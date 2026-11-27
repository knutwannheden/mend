use crate::ApplyPatchError;
use crate::UpdateFileChunk;
use crate::seek_sequence::Match;
use crate::seek_sequence::seek_sequence;
use crate::text_file::Replacement;
use crate::text_file::SourceFile;
use std::ops::Range;

#[cfg(test)]
#[path = "file_update_tests.rs"]
mod tests;

/// Returns the contents that result from applying `chunks` to `original_contents`.
/// `path` only labels messages. The second value notes each chunk that changed more than
/// one place.
pub(crate) fn derive_new_contents_from_chunks(
    path: &str,
    original_contents: &str,
    chunks: &[UpdateFileChunk],
    matching: Matching,
) -> Result<(String, Vec<String>), ApplyPatchError> {
    let mut source_file = SourceFile::parse(original_contents);
    let original_lines = source_file.line_texts();
    let (replacements, notes) = compute_replacements(&original_lines, path, chunks, matching)?;
    source_file.apply_replacements(&replacements);
    Ok((source_file.into_contents(), notes))
}

/// How far a chunk may depart from matching whole lines in exactly one place.
#[derive(Clone, Copy, Debug, Default)]
pub struct Matching {
    /// A chunk of one removed and one added line may replace part of a line, when no whole
    /// line matches.
    pub fragments: bool,
    /// A chunk that matches several places changes all of them.
    pub all: bool,
}

/// Compute a list of replacements needed to transform `original_lines` into the
/// new lines, given the patch `chunks`. Each replacement is returned as
/// `(start_index, old_len, new_lines)`.
///
/// Every chunk has to match exactly one place. It is looked for between its search start
/// and the next anchor of a later chunk, then down to the end of the file, then in the whole
/// file.
/// The first of these that holds a match decides.
fn compute_replacements(
    original_lines: &[String],
    path: &str,
    chunks: &[UpdateFileChunk],
    matching: Matching,
) -> Result<(Vec<Replacement>, Vec<String>), ApplyPatchError> {
    // Each replacement with the line where its chunk matched.
    let mut replacements: Vec<(usize, Replacement)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut line_index: usize = 0;
    // The lines each chunk has matched so far, as (start, length).
    let mut matched: Vec<(usize, usize)> = Vec::new();
    // The parts of lines that fragment chunks replace, as (line, byte range, new text).
    let mut fragments: Vec<(usize, Range<usize>, &str)> = Vec::new();

    for (chunk_index, chunk) in chunks.iter().enumerate() {
        let chunk_label = format!("{path}: chunk {} of {}", chunk_index + 1, chunks.len());

        // An anchor only narrows the search, so its first occurrence is taken. One that lies
        // above the search start is already satisfied, as for a chunk that repeats the
        // anchor of the chunk before it. Then its nearest occurrence above counts.
        let mut anchor_line = None;
        // The chunk's anchor. The chunk has to lie between it and the next anchor.
        let mut section_start = None;
        if let Some(ctx_line) = &chunk.change_context {
            match locate_anchor(original_lines, ctx_line, line_index) {
                Anchor::Below(idx) => {
                    anchor_line = Some(idx);
                    section_start = Some(idx);
                    line_index = idx + 1;
                }
                Anchor::Above(idx) => {
                    anchor_line = Some(idx);
                    section_start = Some(idx);
                }
                Anchor::Missing => {
                    if let Some(adder) = chunks[..chunk_index]
                        .iter()
                        .position(|earlier| adds_line(earlier, ctx_line))
                    {
                        return Err(ApplyPatchError::DoesNotApply(format!(
                            "{chunk_label}: anchor `@@ {ctx_line}` not found\n  \
                             chunk {} adds that line, but an anchor has to name a line the \
                             file has before the patch",
                            adder + 1
                        )));
                    }
                    return Err(ApplyPatchError::DoesNotApply(format!(
                        "{chunk_label}: anchor `@@ {ctx_line}` not found\n  \
                         an anchor has to be a whole line, or the start of exactly one line"
                    )));
                }
                Anchor::Ambiguous(lines) => {
                    return Err(ApplyPatchError::DoesNotApply(format!(
                        "{chunk_label}: anchor `@@ {ctx_line}` is the start of {} lines, at {}\n  \
                         give more of the line so that it starts only one",
                        lines.len(),
                        line_numbers(&lines)
                    )));
                }
            }
        }

        if chunk.old_lines.is_empty() {
            let at = match anchor_line {
                Some(line) if !chunk.is_end_of_file => line + 1,
                _ => original_lines.len(),
            };
            if let (Some(line), Some(last)) = (anchor_line, chunk.new_lines.last())
                && !chunk.is_end_of_file
                && repeats(last, &original_lines[line])
                && opens_block(original_lines, line)
            {
                return Err(repeated_line_error(&chunk_label, line));
            }
            if anchor_line.is_some()
                && !chunk.is_end_of_file
                && copies_what_follows(&chunk.new_lines, &original_lines[at..])
            {
                return Err(copied_lines_error(&chunk_label, at));
            }
            if anchor_line.is_some()
                && !chunk.is_end_of_file
                && splits_comment(original_lines, at, &chunk.new_lines)
            {
                return Err(split_comment_error(&chunk_label, at));
            }
            if anchor_line.is_some()
                && !chunk.is_end_of_file
                && goes_inside_block(original_lines, at, &chunk.new_lines)
            {
                return Err(inside_block_error(&chunk_label, at));
            }
            if replacements
                .iter()
                .any(|&(_, (start, len, _))| start < at && at < start + len)
            {
                return Err(overlap_error(&chunk_label, at));
            }
            replacements.push((at, (at, 0, chunk.new_lines.clone())));
            continue;
        }

        // In many real-world diffs the last element of `old_lines` is an *empty* string
        // representing the terminating newline of the region being replaced. This sentinel
        // is not present in `original_lines` because `SourceFile` stores the terminator on
        // the preceding line rather than as an extra trailing element. If a direct search
        // fails and the pattern ends with an empty string, retry without that final element
        // so modifications touching the end-of-file can be located reliably.
        let mut pattern: &[String] = &chunk.old_lines;
        let mut new_slice: &[String] = &chunk.new_lines;
        let next_anchor = chunks
            .iter()
            .skip(chunk_index + 1)
            .find_map(|later| later.change_context.as_ref())
            .and_then(
                |anchor| match locate_anchor(original_lines, anchor, line_index) {
                    Anchor::Below(idx) => Some(idx),
                    _ => None,
                },
            );
        let below = |pattern: &[String]| {
            let above_next_anchor = match next_anchor {
                Some(end) if !chunk.is_end_of_file => {
                    seek_sequence(&original_lines[..end], pattern, line_index, false)
                }
                _ => Match::None,
            };
            match above_next_anchor {
                Match::None => {
                    seek_sequence(original_lines, pattern, line_index, chunk.is_end_of_file)
                }
                found => found,
            }
        };
        let outside_section = |idx: usize| {
            section_start
                .is_some_and(|anchor| idx <= anchor || next_anchor.is_some_and(|end| idx >= end))
        };
        // A chunk whose own lines include its anchor line is placed with that line on the
        // anchor, so the anchor may be changed, removed or have lines added before it.
        let around = |pattern: &[String]| {
            let anchor = section_start?;
            let named = chunk.change_context.as_deref()?.trim();
            let offset = pattern
                .iter()
                .position(|line| !named.is_empty() && line.trim_start().starts_with(named))?;
            let start = anchor.checked_sub(offset)?;
            let end = start + pattern.len();
            let fits = end <= original_lines.len()
                && (!chunk.is_end_of_file || end == original_lines.len())
                && seek_sequence(&original_lines[start..end], pattern, 0, false) == Match::One(0);
            fits.then_some(start)
        };
        let placed_around = around(pattern);
        let mut found = match placed_around {
            Some(start) => Match::One(start),
            None => below(pattern),
        };
        // Dropping the only line would leave an empty pattern, which matches anywhere.
        if found == Match::None && pattern.len() > 1 && pattern.last().is_some_and(String::is_empty)
        {
            pattern = &pattern[..pattern.len() - 1];
            if new_slice.last().is_some_and(String::is_empty) {
                new_slice = &new_slice[..new_slice.len() - 1];
            }
            found = below(pattern);
        }
        // Of several places, the one that starts right below the anchor is the one it names.
        if let (Match::Many(places), Some(anchor)) = (&found, section_start)
            && !matching.all
            && places.contains(&(anchor + 1))
        {
            found = Match::One(anchor + 1);
        }

        let in_order = found != Match::None;
        if !in_order {
            found = seek_sequence(original_lines, pattern, 0, chunk.is_end_of_file);
        }
        let places_found = match &found {
            Match::One(idx) => vec![*idx],
            Match::Many(found) => found.clone(),
            Match::None => Vec::new(),
        };
        if placed_around.is_none()
            && !places_found.is_empty()
            && places_found.iter().all(|&idx| outside_section(idx))
        {
            return Err(outside_section_error(&chunk_label, chunk, &places_found));
        }
        // For a fragment, the parts of lines it occurs in.
        let mut parts: Vec<(usize, Range<usize>)> = Vec::new();
        let places: Vec<usize> = match found {
            Match::One(start_idx) => vec![start_idx],
            Match::Many(found) if matching.all => {
                let mut apart: Vec<usize> = Vec::new();
                for idx in found {
                    if apart.last().is_none_or(|&last| idx >= last + pattern.len()) {
                        apart.push(idx);
                    }
                }
                apart
            }
            Match::Many(found) => {
                return Err(ApplyPatchError::DoesNotApply(format!(
                    "{chunk_label} matches {} places, at {}\n  \
                     add context lines or an `@@` anchor line so that it matches only one",
                    found.len(),
                    line_numbers(&found)
                )));
            }
            Match::None => {
                let elsewhere = match chunk
                    .is_end_of_file
                    .then(|| seek_sequence(original_lines, pattern, 0, /*eof*/ false))
                {
                    Some(Match::One(idx)) => vec![idx],
                    Some(Match::Many(found)) => found,
                    _ => Vec::new(),
                };
                if !elsewhere.is_empty() {
                    return Err(ApplyPatchError::DoesNotApply(format!(
                        "{chunk_label} matches {}, but not at the end of the file\n  \
                         drop `*** End of File` to change it there",
                        line_numbers(&elsewhere)
                    )));
                }
                let one_pair = pattern.len() == 1
                    && new_slice.len() == 1
                    && chunk.context_line_indices.is_empty()
                    && !pattern[0].trim().is_empty();
                let everywhere = match matching.fragments && one_pair {
                    true => fragment_places(original_lines, &pattern[0]),
                    false => Vec::new(),
                };
                // A fragment is looked for in the same three steps as whole lines.
                let within = |from: usize, to: usize| -> Vec<(usize, usize)> {
                    everywhere
                        .iter()
                        .copied()
                        .filter(|&(line, _)| from <= line && line < to)
                        .collect()
                };
                let steps = [
                    within(line_index, next_anchor.unwrap_or(line_index)),
                    within(line_index, original_lines.len()),
                    everywhere.clone(),
                ];
                let occurrences = steps
                    .into_iter()
                    .find(|step| !step.is_empty())
                    .unwrap_or_default();
                let mut lines: Vec<usize> = occurrences.iter().map(|&(line, _)| line).collect();
                lines.dedup();
                if !lines.is_empty() && lines.iter().all(|&line| outside_section(line)) {
                    return Err(outside_section_error(&chunk_label, chunk, &lines));
                }
                if occurrences.is_empty() {
                    let reason = part_of_a_line(original_lines, pattern)
                        .or_else(|| closest_place(original_lines, pattern));
                    return Err(ApplyPatchError::DoesNotApply(match reason {
                        Some(reason) => format!("{chunk_label} matches nothing\n  {reason}"),
                        None => format!(
                            "{chunk_label} matches nothing, and no similar lines were found"
                        ),
                    }));
                }
                if occurrences.len() > 1 && !matching.all {
                    return Err(ApplyPatchError::DoesNotApply(format!(
                        "{chunk_label}: its line `{}` occurs in {} places, at {}\n  \
                         give more of the line so that it occurs once",
                        shortened(&pattern[0]),
                        occurrences.len(),
                        line_numbers(&lines)
                    )));
                }
                parts = occurrences
                    .iter()
                    .map(|&(line, at)| (line, at..at + pattern[0].len()))
                    .collect();
                lines
            }
        };
        if parts.is_empty()
            && let (Some(&start_idx), Some(context)) =
                (places.first(), repeated_context_line(chunk, new_slice))
            && opens_block(original_lines, start_idx + context)
        {
            return Err(repeated_line_error(&chunk_label, start_idx + context));
        }
        // The added lines that end the chunk, when they follow its last line and replace none.
        let trailing = chunk
            .context_line_indices
            .iter()
            .rfind(|&&(_, new)| new < new_slice.len())
            .filter(|&&(old, _)| old + 1 == pattern.len())
            .map(|&(_, new)| &new_slice[new + 1..]);
        if parts.is_empty()
            && let (Some(&start_idx), Some(trailing)) = (places.first(), trailing)
            && let at = start_idx + pattern.len()
            && copies_what_follows(trailing, &original_lines[at..])
        {
            return Err(copied_lines_error(&chunk_label, at));
        }
        for &start_idx in &places {
            let end_idx = start_idx + pattern.len();
            let whole_lines = parts.is_empty();
            let taken = matched
                .iter()
                .any(|&(start, len)| start_idx < start + len && start < end_idx)
                || whole_lines
                    && fragments
                        .iter()
                        .any(|&(line, ..)| start_idx <= line && line < end_idx);
            if taken {
                return Err(ApplyPatchError::DoesNotApply(format!(
                    "{chunk_label} matches only line {}, which an earlier chunk already matched",
                    start_idx + 1
                )));
            }
            if whole_lines {
                matched.push((start_idx, pattern.len()));
                let indented = indented(&original_lines[start_idx..], pattern, new_slice);
                let new_lines = indented.as_deref().unwrap_or(new_slice);
                let pushed = replacements.len();
                push_replacements(&mut replacements, chunk, pattern, new_lines, start_idx);
                if let Some(&(_, (at, _, _))) =
                    replacements[pushed..]
                        .iter()
                        .find(|(_, (at, old_len, added))| {
                            *old_len == 0 && splits_comment(original_lines, *at, added)
                        })
                {
                    return Err(split_comment_error(&chunk_label, at));
                }
                if let Some(&(_, (at, _, _))) =
                    replacements[pushed..]
                        .iter()
                        .find(|(_, (at, old_len, added))| {
                            *old_len == 0 && goes_inside_block(original_lines, *at, added)
                        })
                {
                    return Err(inside_block_error(&chunk_label, at));
                }
                let replaced = &replacements[pushed..];
                if let Some(&(_, (at, _, _))) =
                    replacements[..pushed]
                        .iter()
                        .find(|&&(_, (at, old_len, _))| {
                            old_len == 0
                                && replaced
                                    .iter()
                                    .any(|&(_, (start, len, _))| start < at && at < start + len)
                        })
                {
                    return Err(overlap_error(&chunk_label, at));
                }
            }
        }
        // A fragment may occur several times on one line, so its places are its parts.
        let changed = parts.len().max(places.len());
        for (line, part) in parts {
            if fragments
                .iter()
                .any(|(at, done, _)| *at == line && done.start < part.end && part.start < done.end)
            {
                return Err(ApplyPatchError::DoesNotApply(format!(
                    "{chunk_label} changes part of line {} that an earlier chunk already changed",
                    line + 1
                )));
            }
            fragments.push((line, part, &new_slice[0]));
        }
        if changed > 1 {
            notes.push(format!(
                "{chunk_label} changed {} places, at {}",
                changed,
                line_numbers(&places)
            ));
        }
        // A chunk found above the search start leaves the search where it was.
        if in_order && let Some(last) = places.last() {
            line_index = line_index.max(last + pattern.len());
        }
    }

    fragments.sort_by_key(|(line, part, _)| (*line, part.start));
    for same_line in fragments.chunk_by(|a, b| a.0 == b.0) {
        let line = same_line[0].0;
        let whole = &original_lines[line];
        let mut composed = String::new();
        let mut from = 0;
        for (_, part, new) in same_line {
            composed.push_str(&whole[from..part.start]);
            composed.push_str(new);
            from = part.end;
        }
        composed.push_str(&whole[from..]);
        replacements.push((line, (line, 1, vec![composed])));
    }

    // Replacements that start at the same line are applied with insertions first, then by
    // where their chunk matched, so that lines added there end up above the lines that
    // replace it, whichever chunk the patch names first.
    replacements.sort_by_key(|(chunk_start, (index, old_len, _))| (*index, *old_len, *chunk_start));

    let replacements = replacements
        .into_iter()
        .map(|(_, replacement)| replacement)
        .collect();
    Ok((replacements, notes))
}

/// Whether an added line copies `existing`, a file line next to where it goes. Writing the line
/// an insertion should go before as context, and then again at the end of the added lines, is a
/// common mistake. Lines without a letter or digit, such as `}`, repeat legitimately.
fn repeats(added: &str, existing: &str) -> bool {
    added.trim() == existing.trim() && added.chars().any(char::is_alphanumeric)
}

/// Whether added lines begin with a copy of the first two of the `following` lines, as when
/// lines already in the file are retyped as if they were context. One copied line is often a
/// sibling's first line, such as an opening tag.
fn copies_what_follows(added: &[String], following: &[String]) -> bool {
    added.len() >= 2 && following.len() >= 2 && (0..2).all(|i| repeats(&added[i], &following[i]))
}

/// Returns the index in `old_lines` of the context line that the added lines ending the chunk
/// follow and end by repeating. Added lines with a context line after them are placed as meant.
fn repeated_context_line(chunk: &UpdateFileChunk, new_slice: &[String]) -> Option<usize> {
    let &(old, new) = chunk
        .context_line_indices
        .iter()
        .rfind(|&&(_, new)| new < new_slice.len())?;
    let last = new_slice.last()?;
    (new_slice.len() > new + 1 && repeats(last, &new_slice[new])).then_some(old)
}

fn repeated_line_error(chunk_label: &str, line: usize) -> ApplyPatchError {
    let line = line + 1;
    ApplyPatchError::DoesNotApply(format!(
        "{chunk_label} repeats line {line} at the end of its added lines\n  \
         to insert before line {line}, give it as a context line after the `+` lines"
    ))
}

/// Whether lines `added` at `at` go between two lines of one comment without being comment
/// lines themselves, as when an insertion meant to go before a doc comment is given its first
/// line as context.
fn splits_comment(lines: &[String], at: usize, added: &[String]) -> bool {
    let comment = |line: &str| {
        let line = line.trim_start();
        line.starts_with("//") || line.starts_with("# ") || line.starts_with("-- ")
    };
    at > 0
        && lines.get(at).is_some_and(|below| comment(below))
        && comment(&lines[at - 1])
        && added.first().is_some_and(|first| !comment(first))
}

/// Whether the line at `idx` opens a block, one the next line is indented deeper than. Lines
/// added below it at its own level were meant to go before it, whereas a block's last line, such
/// as a closing tag, is legitimately repeated by a sibling added after it.
fn opens_block(lines: &[String], idx: usize) -> bool {
    let indent = |line: &str| line.len() - line.trim_start().len();
    match (lines.get(idx), lines.get(idx + 1)) {
        (Some(line), Some(next)) => !next.trim().is_empty() && indent(next) > indent(line),
        _ => false,
    }
}

/// Whether lines `added` at `at` go right below a line that opens a block without being indented
/// deeper themselves. They then cannot be the block's contents.
fn goes_inside_block(lines: &[String], at: usize, added: &[String]) -> bool {
    let indent = |line: &str| line.len() - line.trim_start().len();
    let Some(opener) = at.checked_sub(1).filter(|&idx| opens_block(lines, idx)) else {
        return false;
    };
    let first = added.iter().find(|line| !line.trim().is_empty());
    first.is_some_and(|first| indent(first) <= indent(&lines[opener]))
}

fn inside_block_error(chunk_label: &str, at: usize) -> ApplyPatchError {
    ApplyPatchError::DoesNotApply(format!(
        "{chunk_label} puts its added lines inside the block line {at} opens\n  \
         to insert before it, give its first line, or that of an attribute or comment above it, \
         as a context line after the `+` lines"
    ))
}

fn split_comment_error(chunk_label: &str, at: usize) -> ApplyPatchError {
    ApplyPatchError::DoesNotApply(format!(
        "{chunk_label} puts its added lines inside the comment at {}\n  \
         to insert before the comment, give its first line as a context line after the `+` lines",
        line_numbers(&[at - 1, at])
    ))
}

/// `at` is the index of the line the added lines would go before.
fn overlap_error(chunk_label: &str, at: usize) -> ApplyPatchError {
    ApplyPatchError::DoesNotApply(format!(
        "{chunk_label} overlaps an earlier chunk: lines added between lines {at} and {} would go \
         inside lines the other replaces\n  merge the two chunks into one",
        at + 1
    ))
}

fn outside_section_error(
    chunk_label: &str,
    chunk: &UpdateFileChunk,
    lines: &[usize],
) -> ApplyPatchError {
    ApplyPatchError::DoesNotApply(format!(
        "{chunk_label} matches only outside the lines its anchor `@@ {}` names, at {}\n  \
         an anchored chunk has to lie below its anchor and above the next anchor",
        chunk.change_context.as_deref().unwrap_or_default(),
        line_numbers(lines)
    ))
}

/// `at` is the index of the first of the copied lines.
fn copied_lines_error(chunk_label: &str, at: usize) -> ApplyPatchError {
    ApplyPatchError::DoesNotApply(format!(
        "{chunk_label} begins its added lines with a copy of {}, which follow them\n  \
         to add lines further down, give the last line before them as context",
        line_numbers(&[at, at + 1])
    ))
}

/// Whether `chunk` adds a line that `anchor` would name, as the whole line or its start.
fn adds_line(chunk: &UpdateFileChunk, anchor: &str) -> bool {
    let anchor = anchor.trim();
    !anchor.is_empty()
        && chunk
            .new_lines
            .iter()
            .any(|line| line.trim_start().starts_with(anchor))
}

/// The added lines with the indentation that the chunk's lines leave out of the `lines` they
/// matched, when every one leaves out the same. A patch that drops the indentation of the
/// lines it replaces drops it from its added lines too.
fn indented(lines: &[String], pattern: &[String], new_slice: &[String]) -> Option<Vec<String>> {
    let indent = |s: &str| s.len() - s.trim_start().len();
    let mut left_out: Option<&str> = None;
    for (line, pat) in lines
        .iter()
        .zip(pattern)
        .filter(|(_, pat)| !pat.trim().is_empty())
    {
        let extra = line[..indent(line)].strip_suffix(&pat[..indent(pat)])?;
        if left_out.is_some_and(|seen| seen != extra) {
            return None;
        }
        left_out = Some(extra);
    }
    let left_out = left_out.filter(|extra| !extra.is_empty())?;
    let indented = new_slice.iter().map(|line| match line.trim().is_empty() {
        true => line.clone(),
        false => format!("{left_out}{line}"),
    });
    Some(indented.collect())
}

/// Adds the replacements that turn the lines a chunk matched at `start_idx` into its new
/// lines.
///
/// Context lines occur on both sides of a chunk. They stay in place, so their exact contents
/// and line endings survive, especially in a file with mixed line endings.
fn push_replacements(
    replacements: &mut Vec<(usize, Replacement)>,
    chunk: &UpdateFileChunk,
    pattern: &[String],
    new_slice: &[String],
    start_idx: usize,
) {
    let mut old_start = 0;
    let mut new_start = 0;
    for &(old_context, new_context) in &chunk.context_line_indices {
        // A trailing empty context line can have been dropped from `pattern` and `new_slice`
        // where it stands for the final newline.
        if old_context >= pattern.len() || new_context >= new_slice.len() {
            break;
        }
        if old_start != old_context || new_start != new_context {
            replacements.push((
                start_idx,
                (
                    start_idx + old_start,
                    old_context - old_start,
                    new_slice[new_start..new_context].to_vec(),
                ),
            ));
        }
        old_start = old_context + 1;
        new_start = new_context + 1;
    }
    if old_start != pattern.len() || new_start != new_slice.len() {
        replacements.push((
            start_idx,
            (
                start_idx + old_start,
                pattern.len() - old_start,
                new_slice[new_start..].to_vec(),
            ),
        ));
    }
}

/// Where an `@@` anchor stands, relative to the line the search starts at.
enum Anchor {
    /// Its first occurrence at or below the search start.
    Below(usize),
    /// Its nearest occurrence, when it occurs only above the search start.
    Above(usize),
    Missing,
    /// The lines it is the start of, when it is no whole line and starts several.
    Ambiguous(Vec<usize>),
}

/// Finds an anchor as a whole line, or failing that as the start of exactly one line. A
/// method signature is often given only up to its opening parenthesis.
fn locate_anchor(lines: &[String], anchor: &str, from: usize) -> Anchor {
    let whole = [anchor.to_string()];
    match seek_sequence(lines, &whole, from, /*eof*/ false) {
        Match::One(idx) => return Anchor::Below(idx),
        Match::Many(found) => return Anchor::Below(found[0]),
        Match::None => {}
    }
    match seek_sequence(lines, &whole, 0, /*eof*/ false) {
        Match::One(idx) => return Anchor::Above(idx),
        Match::Many(found) => {
            return found
                .last()
                .map_or(Anchor::Missing, |&idx| Anchor::Above(idx));
        }
        Match::None => {}
    }
    let start = anchor.trim();
    let starting: Vec<usize> = (0..lines.len())
        .filter(|&idx| !start.is_empty() && lines[idx].trim_start().starts_with(start))
        .collect();
    match starting.as_slice() {
        [] => Anchor::Missing,
        &[idx] if idx >= from => Anchor::Below(idx),
        &[idx] => Anchor::Above(idx),
        _ => Anchor::Ambiguous(starting),
    }
}

/// Formats zero-based line indices as `line 3` or `lines 3, 7`.
fn line_numbers(indices: &[usize]) -> String {
    let numbers: Vec<String> = indices.iter().map(|idx| (idx + 1).to_string()).collect();
    let noun = if numbers.len() == 1 { "line" } else { "lines" };
    format!("{noun} {}", numbers.join(", "))
}

/// Names the one line that holds a chunk line without being equal to it, which is what a
/// chunk gets when it gives only a fragment of a long line.
fn part_of_a_line(lines: &[String], pattern: &[String]) -> Option<String> {
    pattern
        .iter()
        .map(|line| line.trim())
        .filter(|fragment| !fragment.is_empty())
        .find_map(|fragment| {
            if lines.iter().any(|line| line.trim() == fragment) {
                return None;
            }
            let mut holders = (1..).zip(lines).filter(|(_, line)| line.contains(fragment));
            let (number, _) = holders.next()?;
            if holders.next().is_some() {
                return None;
            }
            Some(format!(
                "its line `{}` is only part of line {number}\n  \
                 a chunk line has to be the whole line\n  \
                 with --fragment, a chunk of one `-` and one `+` line may give a part of one",
                shortened(fragment)
            ))
        })
}

/// Every place where `fragment` occurs inside a line, as (line index, byte offset).
fn fragment_places(lines: &[String], fragment: &str) -> Vec<(usize, usize)> {
    lines
        .iter()
        .enumerate()
        .flat_map(|(index, line)| line.match_indices(fragment).map(move |(at, _)| (index, at)))
        .collect()
}

/// The start of a chunk line, for a message that has no room for a long one.
fn shortened(line: &str) -> String {
    const SHOWN: usize = 60;

    let mut shown: String = line.chars().take(SHOWN).collect();
    if line.chars().count() > SHOWN {
        shown.push('…');
    }
    shown
}

/// Describes where in `lines` the `pattern` comes closest to matching, or returns `None`
/// when no place shares at least half of the pattern's text.
fn closest_place(lines: &[String], pattern: &[String]) -> Option<String> {
    let significant = |pat: &&String| !pat.trim().is_empty();
    let weight = pattern.iter().filter(significant).count();
    let score = |start: usize| -> f64 {
        pattern
            .iter()
            .enumerate()
            .filter(|(_, pat)| significant(pat))
            .filter_map(|(offset, pat)| Some(similarity(lines.get(start + offset)?, pat)))
            .sum()
    };
    let (start, best) = (0..lines.len())
        .map(|start| (start, score(start)))
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))?;
    if weight == 0 || best / (weight as f64) < 0.5 {
        return None;
    }

    let same = |offset: usize| {
        lines
            .get(start + offset)
            .is_some_and(|line| line.trim() == pattern[offset].trim())
    };
    let equal = (0..pattern.len()).filter(|&offset| same(offset)).count();
    let differs = (0..pattern.len()).find(|&offset| !same(offset))?;
    let file_line = lines.get(start + differs);
    let (column, patch_shown, file_shown) =
        match file_line.and_then(|line| differing_part(&pattern[differs], line)) {
            Some((column, patch_part, file_part)) => {
                (format!(" at column {column}"), patch_part, file_part)
            }
            None => (
                String::new(),
                pattern[differs].clone(),
                file_line.map_or("<end of file>".to_string(), String::clone),
            ),
        };
    Some(format!(
        "closest is at line {} ({equal} of {} lines match); line {} differs{column}:\n    \
         patch: {patch_shown}\n    \
         file:  {file_shown}",
        start + 1,
        pattern.len(),
        start + differs + 1,
    ))
}

/// Cuts two long lines down to the stretch where they differ and a little of what surrounds
/// it, and gives the column where that stretch starts. Returns `None` for lines short enough
/// to compare by eye.
fn differing_part(patch_line: &str, file_line: &str) -> Option<(usize, String, String)> {
    const LONG_LINE: usize = 120;
    const KEPT_AROUND: usize = 30;

    let (patch, file): (Vec<char>, Vec<char>) =
        (patch_line.chars().collect(), file_line.chars().collect());
    if patch.len().max(file.len()) <= LONG_LINE {
        return None;
    }
    let prefix = patch.iter().zip(&file).take_while(|(a, b)| a == b).count();
    let suffix = patch
        .iter()
        .rev()
        .zip(file.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(patch.len().min(file.len()) - prefix);
    let from = prefix.saturating_sub(KEPT_AROUND);
    let part = |line: &[char]| {
        let to = line.len() - suffix.saturating_sub(KEPT_AROUND);
        let shown: String = line[from..to].iter().collect();
        let (before, after) = (
            if from > 0 { "…" } else { "" },
            if to < line.len() { "…" } else { "" },
        );
        format!("{before}{shown}{after}")
    };
    Some((prefix + 1, part(&patch), part(&file)))
}

/// The share of the longer line that the two lines have in common at their start and end.
fn similarity(line: &str, pat: &str) -> f64 {
    let (line, pat) = (line.trim(), pat.trim());
    if line == pat {
        return 1.0;
    }
    let (line_len, pat_len) = (line.chars().count(), pat.chars().count());
    let prefix = line
        .chars()
        .zip(pat.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = line
        .chars()
        .rev()
        .zip(pat.chars().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(line_len.min(pat_len) - prefix);
    (prefix + suffix) as f64 / line_len.max(pat_len) as f64
}
