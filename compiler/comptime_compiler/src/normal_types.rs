//! Opaque normal-form construction metadata; no definition body is executed.
use super::*;

#[derive(Debug)]
pub struct Constructor {
    pub head: nir::DeclarationId,
    pub ty: Term,
    pub count: usize,
    pub diverges: bool,
}
impl fmt::Display for Constructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(normal-type (head {}) (type {}) (inputs {}) (diverges {}))",
            self.head, self.ty, self.count, self.diverges
        )
    }
}
