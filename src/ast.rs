
use std::str::FromStr;
use std::fmt::Display;

use anyhow::Result;

#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    Split { direction: Direction, children: Vec<(u8, Layout)> },
    Leaf  { command: Option<String>, terminal: bool }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Direction {
    Horizontal,
    Vertical,
    Tabbed
}

impl Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Direction::Horizontal => "h",
            Direction::Vertical   => "v",
            Direction::Tabbed     => "t"
        };
        write!(f, "{}", s)
    }
}

impl FromStr for Direction {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "h" => Ok(Direction::Horizontal),
            "v" => Ok(Direction::Vertical),
            "t" => Ok(Direction::Tabbed),
            _   => anyhow::bail!("Invalid direction: {}", input)
        }
    }
}

#[cfg(test)] mod tests {
    use super::*;

    #[test]
    fn direction_from_str_horizontal() {
        assert_eq!("h".parse::<Direction>().unwrap(), Direction::Horizontal)
    }

    #[test]
    fn direction_from_str_vertical() {
        assert_eq!("v".parse::<Direction>().unwrap(), Direction::Vertical)
    }

    #[test]
    fn direction_from_str_tabbed() {
        assert_eq!("t".parse::<Direction>().unwrap(), Direction::Tabbed)
    }

    #[test]
    fn direction_from_str_invalid() {
        assert!("x".parse::<Direction>().is_err())
    }

    #[test]
    fn direction_display() {
        assert_eq!(Direction::Horizontal.to_string(), "h");
        assert_eq!(Direction::Vertical.to_string(), "v");
        assert_eq!(Direction::Tabbed.to_string(), "t");
    }
}

