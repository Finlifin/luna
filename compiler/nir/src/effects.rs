//! Stable effect-entry contracts. Error-result ABI is deliberately not an
//! effect mode. These descriptors do not themselves authorize execution.
use crate::{DeclarationId, Term};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abi {
    Direct,
    DirectPassing,
    Capturing,
    Async,
}
impl fmt::Display for Abi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Direct => "direct",
            Self::DirectPassing => "direct-passing",
            Self::Capturing => "capturing",
            Self::Async => "async",
        })
    }
}

/// Representation admitted by the scalar installation-slot resource protocol.
/// This classification is not Copy permission; resource verification must also
/// discharge Copy for represented results before publishing a provider's
/// cleanup schedule. Void has no owned slot payload.
pub fn scalar_passing_signature(signature: &Term) -> bool {
    matches!(signature, Term::Function { parameters, result }
        if parameters.is_empty() && matches!(result.as_ref(), Term::Primitive(_))
            && (crate::program::intrinsic_pointer_value(result)
                || **result == Term::Primitive(crate::Primitive::Void)))
}

/// Complete operation label, including all declaration instantiation inputs.
/// Equal spelling or equal result types cannot establish label identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Operation {
    pub declaration: DeclarationId,
    pub arguments: Vec<Term>,
}
impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(operation (declaration {}) (arguments",
            self.declaration
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        f.write_str("))")
    }
}

/// One installed dispatch entry for an operation. Its pattern arms/guards are
/// checked separately. `total` requires a coverage proof from that checker.
/// A partial entry must retain a route to a matching outer entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub operation: Operation,
    pub abi: Abi,
    pub total: bool,
}
impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(effect-entry {} (abi {}) (total {}))",
            self.operation, self.abi, self.total
        )
    }
}

/// A lexical installation, outermost first in the routing input. Runtime
/// activations of the same installation still have distinct environments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub entries: Vec<Entry>,
}
impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(effect-frame (entries")?;
        for entry in &self.entries {
            write!(f, " {entry}")?;
        }
        f.write_str("))")
    }
}

/// Checked declaration metadata. `signature` describes request arguments and
/// the delivered result, not permission to call the operation as a function.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub declaration: DeclarationId,
    pub group: Option<DeclarationId>,
    pub name: String,
    pub type_parameters: Vec<DeclarationId>,
    pub signature: Term,
    pub abi: Abi,
    pub inline: bool,
    pub requirements: Vec<crate::program::TypePredicate>,
    pub capabilities: Vec<crate::program::TraitRequirement>,
}
impl fmt::Display for Declaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(effect-declaration {} (name ", self.declaration)?;
        crate::display::string(f, &self.name)?;
        f.write_str(") (group")?;
        if let Some(group) = self.group {
            write!(f, " {group}")?;
        }
        f.write_str(") (type-parameters")?;
        for parameter in &self.type_parameters {
            write!(f, " {parameter}")?;
        }
        write!(
            f,
            ") (signature {}) (abi {}) (inline {}) (requires",
            self.signature, self.abi, self.inline
        )?;
        for predicate in &self.requirements {
            write!(f, " {predicate}")?;
        }
        f.write_str(") (capabilities")?;
        for requirement in &self.capabilities {
            write!(f, " {requirement}")?;
        }
        f.write_str("))")
    }
}
