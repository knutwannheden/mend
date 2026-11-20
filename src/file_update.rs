use crate::ApplyPatchError;
use crate::UpdateFileChunk;
use crate::seek_sequence::Match;
use crate::seek_sequence::seek_sequence;
use crate::text_file::Replacement;
use crate::text_file::SourceFile;

#[cfg(test)]
#[path = "file_update_tests.rs"]
mod tests;

/// Returns the contents that result from applying `chunks` to `original_contents`.
/// `path` only labels error messages.
pub(crate) fn derive_new_contents_from_chunks(
    path: &str,
    original_contents: &str,
    chunks: &[UpdateFileChunk],
) -> Result<String, ApplyPatchError> {
    let mut source_file = SourceFile::parse(original_contents);
    let original_lines = source_file.line_texts();
    let replacements = compute_replacements(&original_lines, path, chunks)?;
    source_file.apply_replacements(&replacements);
    Ok(source_file.into_contents())
}

/// Compute a list of replacements needed to transform `original_lines` into the
/// new lines, given the patch `chunks`. Each replacement is returned as
/// `(start_index, old_len, new_lines)`.
///
/// Every chunk has to match exactly one place at or after the end of the chunk before it.
fn compute_replacements(
    original_lines: &[String],
    path: &str,
    chunks: &[UpdateFileChunk],
) -> Result<Vec<Replacement>, ApplyPatchError> {
    let mut replacements: Vec<Replacement> = Vec::new();
    let mut line_index: usize = 0;

    for (chunk_index, chunk) in chunks.iter().enumerate() {
        let chunk_label = format!("{path}: chunk {} of {}", chunk_index + 1, chunks.len());

        // An anchor only narrows the search, so its first occurrence is taken.
        if let Some(ctx_line) = &chunk.change_context {
            line_index = match seek_sequence(
                original_lines,
                std::slice::from_ref(ctx_line),
                line_index,
                /*eof*/ false,
            ) {
                Match::One(idx) => idx + 1,
                Match::Many(found) => found[0] + 1,
                Match::None => {
                    let below = match line_index {
                        0 => String::new(),
                        line => format!(" below line {line}"),
                    };
                    return Err(ApplyPatchError::DoesNotApply(format!(
                        "{chunk_label}: anchor `@@ {ctx_line}` not found{below}"
                    )));
                }
            };
        }

        if chunk.old_lines.is_empty() {
            replacements.push((original_lines.len(), 0, chunk.new_lines.clone()));
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
        let mut found = seek_sequence(original_lines, pattern, line_index, chunk.is_end_of_file);
        if found == Match::None && pattern.last().is_some_and(String::is_empty) {
            pattern = &pattern[..pattern.len() - 1];
            if new_slice.last().is_some_and(String::is_empty) {
                new_slice = &new_slice[..new_slice.len() - 1];
            }
            found = seek_sequence(original_lines, pattern, line_index, chunk.is_end_of_file);
        }

        let start_idx = match found {
            Match::One(start_idx) => start_idx,
            Match::Many(found) => {
                let lines: Vec<String> = found.iter().map(|idx| (idx + 1).to_string()).collect();
                return Err(ApplyPatchError::DoesNotApply(format!(
                    "{chunk_label} matches {} places, at lines {}\n  \
                     add context lines or an `@@` anchor line so that it matches only one",
                    found.len(),
                    lines.join(", ")
                )));
            }
            Match::None => {
                let earlier = match seek_sequence(original_lines, pattern, 0, chunk.is_end_of_file)
                {
                    Match::One(idx) => Some(idx),
                    Match::Many(found) => Some(found[0]),
                    Match::None => None,
                };
                let message = match (earlier, closest_place(original_lines, pattern)) {
                    (Some(idx), _) => format!(
                        "{chunk_label} matches only at line {}, above line {} where its search starts\n  \
                         chunks and `@@` anchors must follow file order",
                        idx + 1,
                        line_index + 1
                    ),
                    (None, Some(closest)) => format!("{chunk_label} matches nothing\n  {closest}"),
                    (None, None) => {
                        format!("{chunk_label} matches nothing, and no similar lines were found")
                    }
                };
                return Err(ApplyPatchError::DoesNotApply(message));
            }
        };

        // Context lines occur in both sides of a patch chunk. Keep those
        // original lines in place so their exact contents and terminators
        // survive, especially when the file has mixed line endings.
        let mut old_start = 0;
        let mut new_start = 0;
        for &(old_context, new_context) in &chunk.context_line_indices {
            // A trailing empty context line can be removed from `pattern`
            // and `new_slice` above when it represents the final newline.
            if old_context >= pattern.len() || new_context >= new_slice.len() {
                break;
            }
            if old_start != old_context || new_start != new_context {
                replacements.push((
                    start_idx + old_start,
                    old_context - old_start,
                    new_slice[new_start..new_context].to_vec(),
                ));
            }
            old_start = old_context + 1;
            new_start = new_context + 1;
        }
        if old_start != pattern.len() || new_start != new_slice.len() {
            replacements.push((
                start_idx + old_start,
                pattern.len() - old_start,
                new_slice[new_start..].to_vec(),
            ));
        }
        line_index = start_idx + pattern.len();
    }

    replacements.sort_by_key(|(index, _, _)| *index);

    Ok(replacements)
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
