mod cli;
mod mcp;
mod scenarios;
#[cfg(unix)]
mod then;
#[cfg(not(target_os = "windows"))]
mod tool;
