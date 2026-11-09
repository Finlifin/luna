//! In-memory request ownership; never a language identity or persistent key.
use super::*;

#[derive(Default)]
pub(super) struct Authority {
    owner: Option<Arc<()>>,
    generation: u64,
}

#[derive(Clone, Debug)]
pub(super) struct Ticket {
    owner: Arc<()>,
    generation: u64,
}

impl Ticket {
    pub(super) fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner) && self.generation == other.generation
    }
}

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(generation {})", self.generation)
    }
}

impl Authority {
    pub(super) fn issue(&mut self) -> Result<Ticket, Error> {
        self.generation = self.generation.checked_add(1).ok_or(Error {
            kind: ErrorKind::InvalidControlFlow,
            source: None,
        })?;
        Ok(Ticket {
            owner: self.owner.get_or_insert_with(|| Arc::new(())).clone(),
            generation: self.generation,
        })
    }

    #[cfg(test)]
    pub(super) fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    #[cfg(test)]
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }
}
