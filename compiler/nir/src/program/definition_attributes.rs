use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Supported foreign ABI calling conventions for `__flurry_abi`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CallingConvention {
    C,
    System,
    Stdcall,
    Win64,
    Sysv64,
}

impl CallingConvention {
    pub fn from_symbol(name: &str) -> Option<Self> {
        match name {
            "c" => Some(Self::C),
            "system" => Some(Self::System),
            "stdcall" => Some(Self::Stdcall),
            "win64" => Some(Self::Win64),
            "sysv64" => Some(Self::Sysv64),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::C => "c",
            Self::System => "system",
            Self::Stdcall => "stdcall",
            Self::Win64 => "win64",
            Self::Sysv64 => "sysv64",
        }
    }
}

impl fmt::Display for CallingConvention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(calling-convention {})", self.as_str())
    }
}

/// A required invocation retains the attribute state of its own source world.
/// Before publication only annotated declarations have an outstanding dependency.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum DefinitionAttributes {
    #[default]
    Empty,
    Pending(Arc<BTreeSet<DeclarationId>>),
    /// Objects frozen for an early reader can be published by the same handle.
    Frozen(Arc<BTreeMap<DeclarationId, Arc<Term>>>),
}

impl DefinitionAttributes {
    pub fn lookup(&self, id: DeclarationId) -> Result<Option<&Term>, DeclarationId> {
        match self {
            Self::Pending(ids) if ids.contains(&id) => Err(id),
            Self::Empty | Self::Pending(_) => Ok(None),
            Self::Frozen(values) => Ok(values.get(&id).map(Arc::as_ref)),
        }
    }

    /// Inspect the completed `__flurry_abi` calling-convention property in its owning world.
    pub fn calling_convention(
        &self,
        id: DeclarationId,
        fuel: &mut usize,
    ) -> Result<Option<CallingConvention>, &'static str> {
        let object = self
            .lookup(id)
            .map_err(|_| "ABI metadata is not available")?;
        let Some(Term::NominalValue { fields, .. }) = object else {
            return Ok(None);
        };
        *fuel = fuel
            .checked_sub(fields.len())
            .ok_or("ABI metadata budget exhausted")?;
        for entry in fields.iter().rev() {
            if let Some((Some("__flurry_abi"), _, value)) = crate::objects::entry_parts(entry) {
                let Term::Symbol(convention) = value else {
                    return Err("invalid __flurry_abi metadata value");
                };
                return CallingConvention::from_symbol(convention.as_ref())
                    .map(Some)
                    .ok_or("unsupported __flurry_abi calling convention");
            }
        }
        Ok(None)
    }

    /// Inspect whether the declaration selects the `.c` calling convention.
    pub fn has_c_abi(&self, id: DeclarationId, fuel: &mut usize) -> Result<bool, &'static str> {
        Ok(self.calling_convention(id, fuel)? == Some(CallingConvention::C))
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Pending(ids) => ids.len(),
            Self::Frozen(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Display for DefinitionAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("(definition-attributes")?,
            Self::Pending(ids) => {
                f.write_str("(pending-definition-attributes")?;
                for id in ids.iter() {
                    write!(f, " {id}")?;
                }
            }
            Self::Frozen(values) => {
                f.write_str("(definition-attributes")?;
                for (id, object) in values.iter() {
                    write!(f, " (entry {id} {object})")?;
                }
            }
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Primitive;

    fn abi_object(convention: Term) -> Arc<Term> {
        Arc::new(crate::objects::value(vec![crate::objects::entry(
            Some("__flurry_abi"),
            Term::Primitive(Primitive::Symbol),
            convention,
        )]))
    }

    #[test]
    fn calling_convention_parses_and_formats_as_sexpr() {
        for (sym, expected) in [
            ("c", CallingConvention::C),
            ("system", CallingConvention::System),
            ("stdcall", CallingConvention::Stdcall),
            ("win64", CallingConvention::Win64),
            ("sysv64", CallingConvention::Sysv64),
        ] {
            let parsed = CallingConvention::from_symbol(sym).expect("supported convention");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_str(), sym);
            assert_eq!(parsed.to_string(), format!("(calling-convention {sym})"));
        }
        assert_eq!(CallingConvention::from_symbol("unknown"), None);
    }

    #[test]
    fn definition_attributes_reads_all_conventions_and_enforces_budget() {
        let id = |local| DeclarationId { package: 1, local };
        let frozen = DefinitionAttributes::Frozen(Arc::new(BTreeMap::from([
            (id(1), abi_object(Term::Symbol("c".into()))),
            (id(2), abi_object(Term::Symbol("system".into()))),
            (id(3), abi_object(Term::Symbol("stdcall".into()))),
            (id(4), abi_object(Term::Symbol("win64".into()))),
            (id(5), abi_object(Term::Symbol("sysv64".into()))),
            (id(6), abi_object(Term::Symbol("bogus".into()))),
        ])));
        let mut fuel = 100;
        assert_eq!(
            frozen.calling_convention(id(1), &mut fuel).unwrap(),
            Some(CallingConvention::C)
        );
        assert!(frozen.has_c_abi(id(1), &mut fuel).unwrap());
        assert_eq!(
            frozen.calling_convention(id(2), &mut fuel).unwrap(),
            Some(CallingConvention::System)
        );
        assert!(!frozen.has_c_abi(id(2), &mut fuel).unwrap());
        assert_eq!(
            frozen.calling_convention(id(3), &mut fuel).unwrap(),
            Some(CallingConvention::Stdcall)
        );
        assert_eq!(
            frozen.calling_convention(id(4), &mut fuel).unwrap(),
            Some(CallingConvention::Win64)
        );
        assert_eq!(
            frozen.calling_convention(id(5), &mut fuel).unwrap(),
            Some(CallingConvention::Sysv64)
        );
        assert_eq!(frozen.calling_convention(id(99), &mut fuel).unwrap(), None);
        assert!(frozen.calling_convention(id(6), &mut fuel).is_err());

        let mut zero_fuel = 0;
        assert!(frozen.calling_convention(id(1), &mut zero_fuel).is_err());

        let pending = DefinitionAttributes::Pending(Arc::new(BTreeSet::from([id(1)])));
        assert!(pending.calling_convention(id(1), &mut fuel).is_err());
        assert_eq!(pending.calling_convention(id(2), &mut fuel).unwrap(), None);
    }
}
