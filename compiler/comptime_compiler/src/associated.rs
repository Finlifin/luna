//! Associated member selection retains identity, not an initializer expression.
use super::*;
#[derive(Debug)]
pub struct Access {
    pub interface: nir::program::InterfaceInstance,
    pub subject: Term,
    pub member: nir::DeclarationId,
    pub ty: Term,
}
impl fmt::Display for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-value (interface {}) (subject {}) (member {}) (type {}))",
            self.interface, self.subject, self.member, self.ty
        )
    }
}
