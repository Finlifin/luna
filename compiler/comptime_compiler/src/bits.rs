//! Bit-value operations, independent of memory places or unsafe construction.
use super::*;

#[derive(Clone, Copy, Debug)]
pub enum Operation {
    Slice { start: usize, end: usize },
    Index,
    Window,
    Concat,
    Equal,
    Convert,
    Take,
}

impl Operation {
    pub fn requires_bits(self, index: usize) -> bool {
        !matches!(self, Self::Take) && (index == 0 || matches!(self, Self::Concat | Self::Equal))
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Slice { start, end } => write!(f, "(slice {start} {end})"),
            Self::Index => write!(f, "(index)"),
            Self::Window => write!(f, "(window)"),
            Self::Concat => write!(f, "(concat)"),
            Self::Equal => write!(f, "(equal)"),
            Self::Convert => write!(f, "(convert)"),
            Self::Take => write!(f, "(take)"),
        }
    }
}
