use super::*;
use crate::Sort;
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

pub const STABLE_SINCE_KEY: &str = "__flurry_stable_since";

fn is_canonical_numeric_semver_id(segment: &str) -> bool {
    !segment.is_empty()
        && segment.bytes().all(|byte| byte.is_ascii_digit())
        && (segment == "0" || !segment.starts_with('0'))
}

/// Validate strict SemVer 2.0.0 (`MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]`).
pub fn is_valid_semver(version: &str) -> bool {
    let core_and_pre = match version.split_once('+') {
        Some((head, tail)) => {
            if tail.is_empty()
                || !tail.split('.').all(|id| {
                    !id.is_empty()
                        && id
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })
            {
                return false;
            }
            head
        }
        None => version,
    };
    let core = match core_and_pre.split_once('-') {
        Some((head, tail)) => {
            if tail.is_empty()
                || !tail.split('.').all(|id| {
                    if id.is_empty()
                        || !id
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    {
                        return false;
                    }
                    if id.bytes().all(|byte| byte.is_ascii_digit()) {
                        is_canonical_numeric_semver_id(id)
                    } else {
                        true
                    }
                })
            {
                return false;
            }
            head
        }
        None => core_and_pre,
    };
    let mut parts = core.split('.');
    let (Some(major), Some(minor), Some(patch), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    is_canonical_numeric_semver_id(major)
        && is_canonical_numeric_semver_id(minor)
        && is_canonical_numeric_semver_id(patch)
}

/// Return the declaration head of a nominal or type-level normal-form instance.
pub fn declaration_head_of_type(ty: &Term) -> Option<DeclarationId> {
    match ty {
        Term::Nominal { declaration, .. } => Some(*declaration),
        Term::NormalForm {
            head,
            sort: Sort::Type,
            ..
        } => Some(*head),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectiveAttributeError {
    Pending(DeclarationId),
    MalformedObject(Option<DeclarationId>),
    BudgetExhausted,
    InvalidStableSince {
        declaration: Option<DeclarationId>,
        detail: String,
    },
}

impl fmt::Display for EffectiveAttributeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending(declaration) => {
                write!(f, "(effective-attribute-error (pending {declaration}))")
            }
            Self::MalformedObject(Some(declaration)) => write!(
                f,
                "(effective-attribute-error (malformed-object {declaration}))"
            ),
            Self::MalformedObject(None) => {
                f.write_str("(effective-attribute-error malformed-object)")
            }
            Self::BudgetExhausted => f.write_str("(effective-attribute-error budget-exhausted)"),
            Self::InvalidStableSince {
                declaration: Some(declaration),
                detail,
            } => {
                write!(
                    f,
                    "(effective-attribute-error (invalid-stable-since {declaration} "
                )?;
                crate::display::string(f, detail)?;
                f.write_str("))")
            }
            Self::InvalidStableSince {
                declaration: None,
                detail,
            } => {
                f.write_str("(effective-attribute-error (invalid-stable-since ")?;
                crate::display::string(f, detail)?;
                f.write_str("))")
            }
        }
    }
}

impl std::error::Error for EffectiveAttributeError {}

/// Resolved attribute key/value map inherited from a normal-form definition to a
/// constructed instance, merged with any instance-local overrides (inner-to-outer).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectiveAttributes {
    pub source_declaration: Option<DeclarationId>,
    pub entries: BTreeMap<String, (Term, Term)>,
}

impl EffectiveAttributes {
    /// Merge definition-level attributes on `declaration` with any instance-local
    /// override objects supplied from innermost (`index 0`) to outermost.
    pub fn resolve_declaration(
        attributes: &DefinitionAttributes,
        declaration: Option<DeclarationId>,
        local_overrides_inner_to_outer: &[&Term],
        fuel: &mut usize,
    ) -> Result<Self, EffectiveAttributeError> {
        let mut entries = BTreeMap::new();
        if let Some(id) = declaration {
            let cost = usize::BITS as usize - attributes.len().leading_zeros() as usize + 1;
            let Some(next) = fuel.checked_sub(cost) else {
                *fuel = 0;
                return Err(EffectiveAttributeError::BudgetExhausted);
            };
            *fuel = next;
            if let Some(object) = attributes
                .lookup(id)
                .map_err(EffectiveAttributeError::Pending)?
            {
                Self::merge_object_into(object, Some(id), &mut entries, fuel)?;
            }
        }
        // Apply overrides outer-to-inner so earlier (innermost) overrides win.
        for override_object in local_overrides_inner_to_outer.iter().rev() {
            Self::merge_object_into(override_object, declaration, &mut entries, fuel)?;
        }
        Ok(Self {
            source_declaration: declaration,
            entries,
        })
    }

    /// Propagate attributes from a nominal or normal-form constructor definition to
    /// an instantiated type, applying inner-to-outer instance-local overrides on top.
    pub fn resolve_for_type(
        attributes: &DefinitionAttributes,
        ty: &Term,
        local_overrides_inner_to_outer: &[&Term],
        fuel: &mut usize,
    ) -> Result<Self, EffectiveAttributeError> {
        Self::resolve_declaration(
            attributes,
            declaration_head_of_type(ty),
            local_overrides_inner_to_outer,
            fuel,
        )
    }

    fn merge_object_into(
        object: &Term,
        declaration: Option<DeclarationId>,
        entries: &mut BTreeMap<String, (Term, Term)>,
        fuel: &mut usize,
    ) -> Result<(), EffectiveAttributeError> {
        let Term::NominalValue { ty, fields, .. } = object else {
            return Err(EffectiveAttributeError::MalformedObject(declaration));
        };
        if !matches!(ty.as_ref(), Term::Primitive(crate::Primitive::Object)) {
            return Err(EffectiveAttributeError::MalformedObject(declaration));
        }
        let Some(next) = fuel.checked_sub(fields.len().saturating_add(1)) else {
            *fuel = 0;
            return Err(EffectiveAttributeError::BudgetExhausted);
        };
        *fuel = next;
        for entry in fields {
            let Some((key, domain, value)) = crate::objects::entry_parts(entry) else {
                return Err(EffectiveAttributeError::MalformedObject(declaration));
            };
            if let Some(key) = key {
                entries.insert(key.to_string(), (domain.clone(), value.clone()));
            }
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn has_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn get(&self, key: &str) -> Option<(&Term, &Term)> {
        self.entries.get(key).map(|(domain, value)| (domain, value))
    }

    /// Materialize the merged attribute set back into a canonical NIR `Object` value.
    pub fn to_object_term(&self) -> Term {
        let fields = self
            .entries
            .iter()
            .map(|(key, (domain, value))| {
                crate::objects::entry(Some(key.as_str()), domain.clone(), value.clone())
            })
            .collect();
        crate::objects::value(fields)
    }

    /// Extract and validate `__flurry_stable_since`, if present.
    pub fn stable_since(&self) -> Result<Option<&str>, EffectiveAttributeError> {
        let Some((domain, value)) = self.get(STABLE_SINCE_KEY) else {
            return Ok(None);
        };
        if !matches!(domain, Term::Primitive(crate::Primitive::Str)) {
            return Err(EffectiveAttributeError::InvalidStableSince {
                declaration: self.source_declaration,
                detail: "__flurry_stable_since requires the Str domain".into(),
            });
        }
        let Term::String(version) = value else {
            return Err(EffectiveAttributeError::InvalidStableSince {
                declaration: self.source_declaration,
                detail: "__flurry_stable_since requires a completed str value".into(),
            });
        };
        if !is_valid_semver(version) {
            return Err(EffectiveAttributeError::InvalidStableSince {
                declaration: self.source_declaration,
                detail: format!("invalid SemVer 2.0.0 version `{version}`"),
            });
        }
        Ok(Some(version.as_ref()))
    }
}

impl fmt::Display for EffectiveAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(effective-attributes")?;
        if let Some(declaration) = self.source_declaration {
            write!(f, " (declaration {declaration})")?;
        }
        for (key, (domain, value)) in &self.entries {
            f.write_str(" (entry ")?;
            crate::display::string(f, key)?;
            write!(f, " {domain} {value})")?;
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod inheritance_tests {
    use super::*;
    use crate::{Primitive, Sort};

    fn decl(local: u32) -> DeclarationId {
        DeclarationId { package: 1, local }
    }

    fn obj(entries: Vec<(&str, Term, Term)>) -> Term {
        crate::objects::value(
            entries
                .into_iter()
                .map(|(k, d, v)| crate::objects::entry(Some(k), d, v))
                .collect(),
        )
    }

    #[test]
    fn normal_form_definition_attributes_propagate_and_respect_inner_to_outer_overrides() {
        let nf_head = decl(42);
        let def_obj = obj(vec![
            (
                STABLE_SINCE_KEY,
                Term::Primitive(Primitive::Str),
                Term::String("1.0.0".into()),
            ),
            (
                "align_hint",
                Term::Primitive(Primitive::Str),
                Term::String("align16".into()),
            ),
            (
                "vendor_tag",
                Term::Primitive(Primitive::Str),
                Term::String("core".into()),
            ),
        ]);
        let table =
            DefinitionAttributes::Frozen(Arc::new(BTreeMap::from([(nf_head, Arc::new(def_obj))])));
        let instance = Term::NormalForm {
            head: nf_head,
            sort: Sort::Type,
            arguments: vec![Term::Primitive(Primitive::U32)],
        };
        let outer_override = obj(vec![
            (
                "align_hint",
                Term::Primitive(Primitive::Str),
                Term::String("align32".into()),
            ),
            (
                "vendor_tag",
                Term::Primitive(Primitive::Str),
                Term::String("outer".into()),
            ),
        ]);
        let inner_override = obj(vec![(
            "align_hint",
            Term::Primitive(Primitive::Str),
            Term::String("align64".into()),
        )]);

        let mut fuel = 1000;
        let inherited_only =
            EffectiveAttributes::resolve_for_type(&table, &instance, &[], &mut fuel).unwrap();
        assert_eq!(inherited_only.stable_since().unwrap(), Some("1.0.0"));
        assert_eq!(
            inherited_only.get("align_hint"),
            Some((
                &Term::Primitive(Primitive::Str),
                &Term::String("align16".into())
            ))
        );
        assert_eq!(
            inherited_only.get("vendor_tag"),
            Some((
                &Term::Primitive(Primitive::Str),
                &Term::String("core".into())
            ))
        );

        let merged = EffectiveAttributes::resolve_for_type(
            &table,
            &instance,
            &[&inner_override, &outer_override],
            &mut fuel,
        )
        .unwrap();
        assert_eq!(merged.stable_since().unwrap(), Some("1.0.0"));
        assert_eq!(
            merged.get("align_hint"),
            Some((
                &Term::Primitive(Primitive::Str),
                &Term::String("align64".into())
            ))
        );
        assert_eq!(
            merged.get("vendor_tag"),
            Some((
                &Term::Primitive(Primitive::Str),
                &Term::String("outer".into())
            ))
        );
        assert!(merged.to_string().starts_with("(effective-attributes "));
    }
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
