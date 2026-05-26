
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Direction, Layout};

    // --- detect_ratio -------------------------------------------------------

    #[test]
    fn detect_ratio_none_when_absent() {
        let (rest, r) = detect_ratio("vim").unwrap();
        assert_eq!(rest, "vim");
        assert_eq!(r, None);
    }

    #[test]
    fn detect_ratio_none_without_percent_colon() {
        let (rest, r) = detect_ratio("30rest").unwrap();
        assert_eq!(rest, "30rest");
        assert_eq!(r, None);
    }

    #[test]
    fn detect_ratio_extracts_value() {
        let (rest, r) = detect_ratio("30%:rest").unwrap();
        assert_eq!(rest, "rest");
        assert_eq!(r, Some(30));
    }

    #[test]
    fn detect_ratio_u8_overflow() {
        assert!(detect_ratio("256%:rest").is_err());
    }

    // --- parse_ratio --------------------------------------------------------

    #[test]
    fn parse_ratio_valid() {
        let (rest, r) = parse_ratio("50%:rest").unwrap();
        assert_eq!(rest, "rest");
        assert_eq!(r, Some(50));
    }

    #[test]
    fn parse_ratio_exactly_100() {
        let (_, r) = parse_ratio("100%:rest").unwrap();
        assert_eq!(r, Some(100));
    }

    #[test]
    fn parse_ratio_zero_rejected() {
        assert!(parse_ratio("0%:rest").is_err());
    }

    #[test]
    fn parse_ratio_above_100_rejected() {
        assert!(parse_ratio("101%:rest").is_err());
    }

    // --- parse_leaf ---------------------------------------------------------

    #[test]
    fn parse_leaf_bare_command() {
        let (rest, leaf) = parse_leaf("vim").unwrap();
        assert_eq!(rest, "");
        assert_eq!(leaf, Layout::Leaf { command: Some("vim".into()), terminal: false });
    }

    #[test]
    fn parse_leaf_braced_command() {
        let (rest, leaf) = parse_leaf("{vim}").unwrap();
        assert_eq!(rest, "");
        assert_eq!(leaf, Layout::Leaf { command: Some("vim".into()), terminal: true });
    }

    #[test]
    fn parse_leaf_empty_braces() {
        let (rest, leaf) = parse_leaf("{}").unwrap();
        assert_eq!(rest, "");
        assert_eq!(leaf, Layout::Leaf { command: None, terminal: true });
    }

    #[test]
    fn parse_leaf_stops_at_comma() {
        let (rest, leaf) = parse_leaf("vim, other").unwrap();
        assert_eq!(rest, ", other");
        assert_eq!(leaf, Layout::Leaf { command: Some("vim".into()), terminal: false });
    }

    #[test]
    fn parse_leaf_stops_at_close_paren() {
        let (rest, leaf) = parse_leaf("vim)").unwrap();
        assert_eq!(rest, ")");
        assert_eq!(leaf, Layout::Leaf { command: Some("vim".into()), terminal: false });
    }

    #[test]
    fn parse_leaf_misplaced_ratio_rejected() {
        assert!(parse_leaf("{30%:vim}").is_err());
    }

    #[test]
    fn parse_leaf_unclosed_brace_rejected() {
        assert!(parse_leaf("{vim").is_err());
    }

    #[test]
    fn parse_leaf_command_with_args() {
        let (rest, leaf) = parse_leaf("{tig -w}").unwrap();
        assert_eq!(rest, "");
        assert_eq!(leaf, Layout::Leaf { command: Some("tig -w".into()), terminal: true });
    }

    // --- parse_split --------------------------------------------------------

    #[test]
    fn parse_split_horizontal() {
        let (rest, split) = parse_split("h({vim}, {nvim})").unwrap();
        assert_eq!(rest, "");
        assert_eq!(split, Layout::Split {
            direction: Direction::Horizontal,
            children: vec![
                (0, Layout::Leaf { command: Some("vim".into()),  terminal: true }),
                (0, Layout::Leaf { command: Some("nvim".into()), terminal: true }),
            ],
        });
    }

    #[test]
    fn parse_split_vertical() {
        let (_, split) = parse_split("v({vim}, {nvim})").unwrap();
        assert!(matches!(split, Layout::Split { direction: Direction::Vertical, .. }));
    }

    #[test]
    fn parse_split_tabbed() {
        let (_, split) = parse_split("t({vim}, {nvim})").unwrap();
        assert!(matches!(split, Layout::Split { direction: Direction::Tabbed, .. }));
    }

    #[test]
    fn parse_split_with_ratios() {
        let (rest, split) = parse_split("h(30%:{vim}, {nvim})").unwrap();
        assert_eq!(rest, "");
        assert_eq!(split, Layout::Split {
            direction: Direction::Horizontal,
            children: vec![
                (30, Layout::Leaf { command: Some("vim".into()),  terminal: true }),
                (0,  Layout::Leaf { command: Some("nvim".into()), terminal: true }),
            ],
        });
    }

    #[test]
    fn parse_split_unclosed_paren_rejected() {
        assert!(parse_split("h({vim}").is_err());
    }

    #[test]
    fn parse_split_three_children() {
        let (_, split) = parse_split("h({a}, {b}, {c})").unwrap();
        if let Layout::Split { children, .. } = split {
            assert_eq!(children.len(), 3);
        } else {
            panic!("expected Split");
        }
    }

    // --- parse_layout -------------------------------------------------------

    #[test]
    fn parse_layout_with_ratio() {
        let (rest, (ratio, layout)) = parse_layout("30%:{vim}").unwrap();
        assert_eq!(rest, "");
        assert_eq!(ratio, 30);
        assert_eq!(layout, Layout::Leaf { command: Some("vim".into()), terminal: true });
    }

    #[test]
    fn parse_layout_without_ratio() {
        let (rest, (ratio, _)) = parse_layout("{vim}").unwrap();
        assert_eq!(rest, "");
        assert_eq!(ratio, 0);
    }

    #[test]
    fn parse_layout_leading_whitespace() {
        let (rest, (ratio, _)) = parse_layout("   {vim}").unwrap();
        assert_eq!(rest, "");
        assert_eq!(ratio, 0);
    }

    // --- FromStr for Layout -------------------------------------------------

    #[test]
    fn from_str_bare_leaf() {
        let l: Layout = "vim".parse().unwrap();
        assert_eq!(l, Layout::Leaf { command: Some("vim".into()), terminal: false });
    }

    #[test]
    fn from_str_braced_leaf() {
        let l: Layout = "{vim}".parse().unwrap();
        assert_eq!(l, Layout::Leaf { command: Some("vim".into()), terminal: true });
    }

    #[test]
    fn from_str_empty_braces() {
        let l: Layout = "{}".parse().unwrap();
        assert_eq!(l, Layout::Leaf { command: None, terminal: true });
    }

    #[test]
    fn from_str_simple_horizontal_split() {
        let l: Layout = "h({vim}, {nvim})".parse().unwrap();
        assert_eq!(l, Layout::Split {
            direction: Direction::Horizontal,
            children: vec![
                (0, Layout::Leaf { command: Some("vim".into()),  terminal: true }),
                (0, Layout::Leaf { command: Some("nvim".into()), terminal: true }),
            ],
        });
    }

    #[test]
    fn from_str_nested_splits() {
        let l: Layout = "h(v({vim}, {tig}), {nvim})".parse().unwrap();
        assert_eq!(l, Layout::Split {
            direction: Direction::Horizontal,
            children: vec![
                (0, Layout::Split {
                    direction: Direction::Vertical,
                    children: vec![
                        (0, Layout::Leaf { command: Some("vim".into()), terminal: true }),
                        (0, Layout::Leaf { command: Some("tig".into()), terminal: true }),
                    ],
                }),
                (0, Layout::Leaf { command: Some("nvim".into()), terminal: true }),
            ],
        });
    }

    #[test]
    fn from_str_trailing_content_rejected() {
        assert!("{vim} extra".parse::<Layout>().is_err());
    }

    #[test]
    fn from_str_misplaced_ratio_rejected() {
        assert!("{30%:vim}".parse::<Layout>().is_err());
    }

    #[test]
    fn from_str_zero_ratio_rejected() {
        assert!("0%:{vim}".parse::<Layout>().is_err());
    }

    #[test]
    fn from_str_ratio_above_100_rejected() {
        assert!("101%:{vim}".parse::<Layout>().is_err());
    }

    #[test]
    fn from_str_unclosed_split_rejected() {
        assert!("h({vim}".parse::<Layout>().is_err());
    }

    #[test]
    fn from_str_example_from_comment() {
        let l = "t(h(v(30%:{ranger}, {tig -w}), 50%:{}), h({vim}, {opencode}))".parse::<Layout>();
        assert!(l.is_ok());
        assert!(matches!(l.unwrap(), Layout::Split { direction: Direction::Tabbed, .. }));
    }

    #[test]
    fn from_str_empty_input_rejected() {
        assert!("".parse::<Layout>().is_err());
    }

    #[test]
    fn from_str_whitespace_only_rejected() {
        assert!("   ".parse::<Layout>().is_err());
    }
}

