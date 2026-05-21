
use std::str::FromStr;

use anyhow::Result;
use anyhow::Context;

use crate::ast::{Layout};

// t(h(v(30%:{ranger}, {tig -w}), 50%:{}), h({vim}, {opencode}))
//
// Grammaire:
//   layout = (ratio ':')? node
//   node   = split | feuille
//   split  = ('h'|'v'|'t') '(' layout (',' layout)* ')'
//   leaf   = '{' command? '}' | command
//   ratio  = chiffres '%'
fn parse_layout(input: &str) -> Result<(&str, (u8, Layout))> {
    let trimmed_in         = input.trim_start();
    let (rest, r)          = parse_ratio(trimmed_in)?;
    let (new_rest, layout) = parse_node(rest)?;
    Ok((new_rest, (r.unwrap_or(0), layout)))
}

fn parse_node(input: &str) -> Result<(&str, Layout)> {
    let trimmed_in = input.trim_start();
    if trimmed_in.is_empty() {
        anyhow::bail!("Unexpected end of input while parsing layout");
    }

    match (trimmed_in.chars().next(), trimmed_in.chars().nth(1)) {
        (Some('h') | Some('v') | Some('t'), Some('(')) => parse_split(trimmed_in),
        _                                              => parse_leaf(trimmed_in)
    }
}

fn parse_split(input: &str) -> Result<(&str, Layout)> {
    let trimmed_in = input.trim_start();
    let (d_str, mut rest) = trimmed_in.split_at(1);

    if !rest.starts_with('(') {
        anyhow::bail!("Expected split to be in the format 'd(...)', got: {}", input);
    }

    let direction = d_str.parse().context("Failed to parse split direction")?;

    rest = &rest[1..]; // Remove the opening parenthesis
    let children = std::iter::from_fn(|| {
        rest = rest.trim_start();
        if rest.is_empty() || rest.starts_with(')') {
            return None;
        }

        match parse_layout(rest) {
            Ok((new_rest, (r, layout))) => {
                rest = new_rest.trim_start();
                if rest.starts_with(',') {
                    rest = &rest[1..];
                }
                Some(Ok((r, layout)))
            }
            Err(e) => Some(Err(e))
        }
    }).collect::<Result<Vec<(u8, Layout)>>>()?;

    if rest.starts_with(')') {
        rest = &rest[1..];
    } else {
        anyhow::bail!("Expected ')' to close split");
    }

    Ok((rest, Layout::Split { direction, children }))
}

fn parse_leaf(input: &str) -> Result<(&str, Layout)> {
    let trimmed_in      = input.trim_start();
    let mut brace_depth = 0;
    let mut iter        = trimmed_in.char_indices();
    let (end_of_command_pos, rest_start_pos) = loop {
        match iter.next() {
            Some((_, '{')) => brace_depth += 1,
            Some((i, '}')) => {
                brace_depth -= 1;
                if brace_depth == 0 {
                    break (i, i+1);
                }
            },
            Some((i, ',')) if brace_depth == 0 => break (i, i),
            Some((i, ')')) if brace_depth == 0 => break (i, i),
            Some(_) => {}
            None => {
                if brace_depth != 0 {
                    anyhow::bail!("Unmatched braces or parentheses in leaf command: {}", input);
                }
                break (trimmed_in.len(), trimmed_in.len())
            },
        }
    };
    let is_braced      = trimmed_in.starts_with('{');
    let command_offset = if is_braced { 1 } else { 0 };
    let command        = trimmed_in[command_offset..end_of_command_pos].trim().to_string();

    // Check for ratios in the command. This is to prevent waiting on timeout if the user
    // accidentally writes something like "{30%:vim}" when he really meant "30%:{vim}".
    let misplaced_ratio = detect_ratio(&command);
    if let Ok((_, Some(_))) = misplaced_ratio {
        let (r, c) = command.split_once(':').unwrap();
        anyhow::bail!("Ratios for terminal commands must prefix braces {{...}}. Perhaps you meant {r}:{{{c}}}")
    }

    let leaf = Layout::Leaf { command: (!command.is_empty()).then_some(command), terminal: is_braced };
    Ok((&trimmed_in[rest_start_pos..], leaf))
}

fn detect_ratio(input: &str) -> Result<(&str, Option<u8>)> {
    let leading_numbers_count = input.chars().take_while(|c| c.is_ascii_digit()).count();
    if input[leading_numbers_count..].starts_with("%:") {
        let ratio = input[..leading_numbers_count].parse::<u8>().context("Failed to parse ratio")?;
        Ok((&input[leading_numbers_count + 2..], Some(ratio)))
    } else {
        Ok((input, None))
    }
}

fn parse_ratio(input: &str) -> Result<(&str, Option<u8>)> {
    let (rest, ratio_opt) = detect_ratio(input)?;
    if let Some(r) = ratio_opt && (r == 0 || r > 100) {
        anyhow::bail!("Ratio must be between 1 and 100");
    }
    Ok((rest, ratio_opt))
}

impl FromStr for Layout {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (rest, (_, layout)) = parse_layout(input)?;
        if !rest.trim().is_empty() {
            anyhow::bail!("Unexpected input after parsing layout: '{}'", rest);
        }
        Ok(layout)
    }
}

