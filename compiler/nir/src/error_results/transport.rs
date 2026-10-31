//! Checked, field-preserving representation transport. This plan moves the
//! whole owner; it is not a source-language destructuring operation.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepresentationTransport {
    Identity,
    Error {
        source: Term,
        target: Term,
        tags: Vec<usize>,
    },
    Tuple(Vec<RepresentationTransport>),
    Record(Vec<(String, RepresentationTransport)>),
}

/// Borrow declaration-authorized tuple fields without prescribing their input
/// encoding. Layout evidence never establishes equality between opaque heads.
pub fn transport_plan_with_layouts(
    input: &Term,
    output: &Term,
    exact_rows: bool,
    fuel: &mut usize,
    equal: &mut impl FnMut(&Term, &Term) -> bool,
    tuple_fields: &impl for<'t> Fn(&'t Term) -> Option<&'t [Term]>,
) -> Result<RepresentationTransport, Error> {
    type_input(input, fuel)?;
    type_input(output, fuel)?;
    build(input, output, exact_rows, fuel, equal, tuple_fields, 0)
}
fn build(
    input: &Term,
    output: &Term,
    exact: bool,
    fuel: &mut usize,
    equal: &mut impl FnMut(&Term, &Term) -> bool,
    tuple_fields: &impl for<'t> Fn(&'t Term) -> Option<&'t [Term]>,
    depth: usize,
) -> Result<RepresentationTransport, Error> {
    if depth >= 128 {
        return Err(Error::Budget);
    }
    charge(input, fuel)?;
    charge(output, fuel)?;
    if input == output {
        return Ok(RepresentationTransport::Identity);
    }
    if let (
        Term::ErrorResult {
            errors: a,
            result: ar,
        },
        Term::ErrorResult {
            errors: b,
            result: br,
        },
    ) = (input, output)
    {
        if a.is_empty() || b.is_empty() {
            return Err(Error::InvalidType);
        }
        let mut tags = vec![0];
        for label in a {
            let mut selected = None;
            for (index, candidate) in b.iter().enumerate() {
                charge(label, fuel)?;
                charge(candidate, fuel)?;
                if label == candidate || equal(label, candidate) {
                    selected = Some(index + 1);
                    break;
                }
            }
            tags.push(selected.ok_or(Error::MissingLabel)?);
        }
        if exact {
            for label in b {
                let mut found = false;
                for candidate in a {
                    charge(label, fuel)?;
                    charge(candidate, fuel)?;
                    if label == candidate || equal(label, candidate) {
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(Error::MissingLabel);
                }
            }
        }
        if ar != br
            && !(!exact && ar.as_ref() == &Term::Primitive(crate::Primitive::Never))
            && !equal(ar, br)
        {
            return Err(Error::IncompatibleSuccess);
        }
        return Ok(RepresentationTransport::Error {
            source: input.clone(),
            target: output.clone(),
            tags,
        });
    }
    let tuples = match (input, output) {
        (Term::Tuple(a), Term::Tuple(b)) => Some((a.as_slice(), b.as_slice())),
        (Term::NormalForm { head: a, .. }, Term::NormalForm { head: b, .. }) if a == b => {
            tuple_fields(input).zip(tuple_fields(output))
        }
        _ => None,
    };
    if let Some((a, b)) = tuples {
        if a.len() != b.len() {
            return Err(Error::InvalidType);
        }
        return a
            .iter()
            .zip(b)
            .map(|(a, b)| build(a, b, true, fuel, equal, tuple_fields, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(RepresentationTransport::Tuple);
    }
    if let (Term::Record(a), Term::Record(b)) = (input, output) {
        if a.len() != b.len() {
            return Err(Error::InvalidType);
        }
        let mut fields = Vec::new();
        for ((name, a), (other, b)) in a.iter().zip(b) {
            if name != other {
                return Err(Error::InvalidType);
            }
            fields.push((
                name.clone(),
                build(a, b, true, fuel, equal, tuple_fields, depth + 1)?,
            ));
        }
        return Ok(RepresentationTransport::Record(fields));
    }
    if equal(input, output) {
        Ok(RepresentationTransport::Identity)
    } else {
        Err(Error::InvalidType)
    }
}

impl RepresentationTransport {
    /// Transport an already admitted immutable value. This checks the stored
    /// source domain and branch identity; payload admission still belongs to
    /// the typed constant producer, not to representation conversion.
    pub fn frozen(&self, value: &Term, fuel: &mut usize) -> Result<Term, Error> {
        charge(value, fuel)?;
        value.check_scope(&[]).map_err(|_| Error::InvalidType)?;
        self.frozen_at(value, fuel, 0)
    }

    fn frozen_at(&self, value: &Term, fuel: &mut usize, depth: usize) -> Result<Term, Error> {
        if depth >= 128 {
            return Err(Error::Budget);
        }
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        match (self, value) {
            (Self::Identity, value) => Ok(value.clone()),
            (
                Self::Error {
                    source,
                    target,
                    tags,
                },
                Term::ErrorValue { ty, label, value },
            ) => {
                charge(source, fuel)?;
                charge(target, fuel)?;
                if ty.as_ref() != source {
                    return Err(Error::InvalidType);
                }
                let Term::ErrorResult { errors: from, .. } = source else {
                    return Err(Error::InvalidType);
                };
                let Term::ErrorResult { errors: to, .. } = target else {
                    return Err(Error::InvalidType);
                };
                let tag = if let Some(label) = label {
                    let mut tag = None;
                    for (index, candidate) in from.iter().enumerate() {
                        charge(candidate, fuel)?;
                        charge(label, fuel)?;
                        if candidate == label.as_ref() {
                            tag = Some(index + 1);
                            break;
                        }
                    }
                    tag.ok_or(Error::MissingLabel)?
                } else {
                    0
                };
                let target_tag = *tags.get(tag).ok_or(Error::InvalidTag)?;
                let label = match target_tag.checked_sub(1) {
                    Some(index) => Some(Box::new(to.get(index).ok_or(Error::InvalidTag)?.clone())),
                    None => None,
                };
                Ok(Term::ErrorValue {
                    ty: Box::new(target.clone()),
                    label,
                    value: value.clone(),
                })
            }
            (Self::Tuple(plans), Term::TupleValue(values)) if plans.len() == values.len() => {
                let mut output = Vec::new();
                for (plan, value) in plans.iter().zip(values) {
                    output.push(plan.frozen_at(value, fuel, depth + 1)?);
                }
                Ok(Term::TupleValue(output))
            }
            (Self::Record(plans), Term::RecordValue(values)) if plans.len() == values.len() => {
                let mut output = Vec::new();
                for ((name, plan), (actual, value)) in plans.iter().zip(values) {
                    *fuel = fuel.checked_sub(name.len()).ok_or(Error::Budget)?;
                    if name != actual {
                        return Err(Error::InvalidType);
                    }
                    output.push((name.clone(), plan.frozen_at(value, fuel, depth + 1)?));
                }
                Ok(Term::RecordValue(output))
            }
            _ => Err(Error::InvalidType),
        }
    }

    fn locate<'a>(&'a self, path: &[usize], fuel: &mut usize) -> Result<(&'a Self, usize), Error> {
        let mut node = self;
        let mut offset = 0;
        loop {
            *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
            if offset == path.len() {
                return Ok((node, offset));
            }
            node = match node {
                Self::Tuple(fields) => fields.get(path[offset]).ok_or(Error::InvalidType)?,
                Self::Record(fields) => &fields.get(path[offset]).ok_or(Error::InvalidType)?.1,
                _ => return Ok((node, offset)),
            };
            offset += 1;
        }
    }
    pub fn remap_path(&self, path: &mut [usize], fuel: &mut usize) -> Result<(), Error> {
        let (node, offset) = self.locate(path, fuel)?;
        let path = &mut path[offset..];
        if let Self::Error { tags, .. } = node {
            if !path.is_empty() {
                if path.len() < 3 || path[0] != usize::MAX || path[2] != 0 {
                    return Err(Error::InvalidType);
                }
                path[1] = *tags.get(path[1]).ok_or(Error::InvalidType)?;
            }
        }
        Ok(())
    }
    /// Domain/guard paths name branch containers rather than payload leaves.
    pub fn remap_tag(&self, path: &[usize], tag: usize, fuel: &mut usize) -> Result<usize, Error> {
        let (node, offset) = self.locate(path, fuel)?;
        match node {
            Self::Error { tags, .. } if offset == path.len() => {
                tags.get(tag).copied().ok_or(Error::InvalidType)
            }
            _ => Ok(tag),
        }
    }
    pub fn inverse_path(&self, path: &mut [usize], fuel: &mut usize) -> Result<bool, Error> {
        let (node, offset) = self.locate(path, fuel)?;
        let path = &mut path[offset..];
        if let Self::Error { tags, .. } = node {
            if !path.is_empty() {
                if path.len() < 3 || path[0] != usize::MAX || path[2] != 0 {
                    return Err(Error::InvalidType);
                }
                *fuel = fuel.checked_sub(tags.len()).ok_or(Error::Budget)?;
                let Some(index) = tags.iter().position(|tag| *tag == path[1]) else {
                    return Ok(false);
                };
                path[1] = index;
            }
        }
        Ok(true)
    }
}

impl fmt::Display for RepresentationTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Identity => f.write_str("(identity)"),
            Self::Error {
                source,
                target,
                tags,
            } => {
                write!(f, "(error-layout (source {source}) (target {target}) (tags")?;
                for tag in tags {
                    write!(f, " {tag}")?;
                }
                f.write_str("))")
            }
            Self::Tuple(fields) => {
                f.write_str("(tuple-layout")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str(")")
            }
            Self::Record(fields) => {
                f.write_str("(record-layout")?;
                for (name, field) in fields {
                    write!(f, " (field {name:?} {field})")?;
                }
                f.write_str(")")
            }
        }
    }
}
