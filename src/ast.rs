
use std::str::FromStr;
use anyhow::Result;

#[derive(Clone)]
pub enum Layout {
    Split { direction: Direction, children: Vec<(u8, Layout)> },
    Leaf  { command: Option<String>, terminal: bool }
}

#[derive(Clone)]
pub enum Direction {
    Horizontal,
    Vertical,
    Tabbed
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

