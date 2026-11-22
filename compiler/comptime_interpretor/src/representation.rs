//! Inspect immutable host data with the observer's fuel. List summaries carry
//! structural facts about a payload, never evidence of membership in a domain.
use super::*;
use type_descriptions::spend;

const MAX_SIZE: usize = 100_000;
const MAX_DEPTH: usize = 128;

/// Size of an inspected value for full-content comparison and host cloning.
/// These counts grant neither source Eq/Clone nor callable authority.
#[derive(Debug, PartialEq, Eq)]
pub struct ImmutableSize {
    pub expanded: usize,
    pub owned: usize,
}

impl Value {
    pub fn inspect_immutable(&self, fuel: &mut usize) -> Result<ImmutableSize, ErrorKind> {
        self.inspect_representation(&BTreeSet::new(), &BTreeMap::new(), &BTreeMap::new(), fuel)
    }

    pub(super) fn inspect_representation(
        &self,
        parameters: &BTreeSet<DeclarationId>,
        value_parameters: &BTreeMap<DeclarationId, Term>,
        runtime_domains: &BTreeMap<nir::RuntimeId, Term>,
        fuel: &mut usize,
    ) -> Result<ImmutableSize, ErrorKind> {
        let mut walk = Inspection {
            parameters,
            value_parameters,
            runtime_domains,
            fuel,
            expanded: 0,
            shared: 0,
        };
        walk.visit(self, 0, 0)?;
        Ok(ImmutableSize {
            expanded: walk.expanded,
            owned: walk.expanded - walk.shared,
        })
    }

    /// Unmetered host estimate. Execution paths supply fuel to inspect_immutable.
    pub fn immutable_copy_work(&self) -> Option<usize> {
        let mut fuel = usize::MAX;
        self.inspect_immutable(&mut fuel).ok().map(|s| s.expanded)
    }

    pub fn immutable_clone_work(&self) -> Option<usize> {
        let mut fuel = usize::MAX;
        let size = self.inspect_immutable(&mut fuel).ok()?;
        Some(usize::MAX - fuel + size.owned)
    }

    #[cfg(test)]
    pub(super) fn scoped_copy_work(&self, parameters: &BTreeSet<DeclarationId>) -> Option<usize> {
        let mut fuel = usize::MAX;
        Some(
            self.inspect_representation(parameters, &BTreeMap::new(), &BTreeMap::new(), &mut fuel)
                .ok()?
                .expanded,
        )
    }
}

struct Inspection<'a> {
    parameters: &'a BTreeSet<DeclarationId>,
    value_parameters: &'a BTreeMap<DeclarationId, Term>,
    runtime_domains: &'a BTreeMap<nir::RuntimeId, Term>,
    fuel: &'a mut usize,
    expanded: usize,
    shared: usize,
}

impl Inspection<'_> {
    fn include(&mut self, amount: usize) -> Result<(), ErrorKind> {
        self.expanded = self.expanded.saturating_add(amount);
        self.reserve_children(0)
    }

    fn reserve_children(&self, pending: usize) -> Result<(), ErrorKind> {
        if self.expanded.saturating_add(pending) > MAX_SIZE {
            return Err(ErrorKind::RepresentationSizeLimit);
        }
        Ok(())
    }

    fn payload(&mut self, amount: usize) -> Result<(), ErrorKind> {
        self.include(amount)?;
        spend(self.fuel, amount)
    }

    fn term(&mut self, term: &Term) -> Result<(), ErrorKind> {
        let size = type_descriptions::inspect_data_term(
            term,
            self.parameters,
            self.value_parameters,
            self.runtime_domains,
            self.fuel,
        )?;
        self.include(size)
    }

    fn domain(&mut self, ty: &Term) -> Result<(), ErrorKind> {
        if ty.sort() != nir::Sort::Type {
            return Err(ErrorKind::Type);
        }
        self.term(ty)
    }

    fn children<'v, T: 'v>(
        &mut self,
        values: impl DoubleEndedIterator<Item = &'v T> + ExactSizeIterator,
        depth: usize,
        siblings: usize,
        mut project: impl FnMut(&mut Self, &'v T) -> Result<&'v Value, ErrorKind>,
    ) -> Result<usize, ErrorKind> {
        self.reserve_children(siblings.saturating_add(values.len()))?;
        let mut height = 0;
        for (remaining, value) in values.enumerate().rev() {
            let child = project(self, value)?;
            height = height.max(self.visit(child, depth + 1, siblings + remaining)?);
        }
        Ok(height)
    }

    fn visit(&mut self, value: &Value, depth: usize, siblings: usize) -> Result<usize, ErrorKind> {
        if depth >= MAX_DEPTH {
            return Err(ErrorKind::DepthExhausted);
        }
        self.include(1)?;
        spend(self.fuel, 1)?;
        let height = match value {
            Value::List { ty, values } => {
                self.domain(ty)?;
                let before = self.expanded;
                let shared = self.shared;
                let height = if let Some(summary) = values.summary() {
                    if depth + summary.height >= MAX_DEPTH {
                        return Err(ErrorKind::DepthExhausted);
                    }
                    self.include(summary.work)?;
                    summary.height
                } else {
                    self.reserve_children(siblings.saturating_add(values.len()))?;
                    let mut scalars = values.first().and_then(list_values::scalar_shape);
                    let mut height = 0;
                    for (remaining, child) in values.iter().enumerate().rev() {
                        height = height.max(self.visit(child, depth + 1, siblings + remaining)?);
                        scalars =
                            scalars.and_then(|seen| seen.join(list_values::scalar_shape(child)?));
                    }
                    if (self.parameters.is_empty()
                        && self.value_parameters.is_empty()
                        && self.runtime_domains.is_empty())
                        || values.is_empty()
                        || scalars.is_some()
                    {
                        values.remember(list_values::Summary {
                            work: self.expanded - before,
                            height,
                            scalars,
                        });
                    }
                    height
                };
                // Cloning the persistent list shares the entire payload, even on
                // the first inspection and even if it contains other shared data.
                self.shared = shared + (self.expanded - before);
                height
            }
            Value::Tuple(fields) => {
                self.children(fields.iter(), depth, siblings, |_, value| Ok(value))?
            }
            Value::AnonymousRecord(fields) => {
                self.children(fields.iter(), depth, siblings, |walk, (name, child)| {
                    walk.payload(name.len())?;
                    Ok(child)
                })?
            }
            Value::Object(object) => {
                self.children(object.entries().iter(), depth, siblings, |walk, entry| {
                    walk.domain(&entry.value().ty)?;
                    walk.payload(entry.key().map_or(0, str::len))?;
                    Ok(&entry.value().value)
                })?
            }
            Value::Record {
                arguments, fields, ..
            }
            | Value::Variant {
                arguments, fields, ..
            } => {
                for term in arguments {
                    self.term(term)?;
                }
                self.children(fields.iter(), depth, siblings, |_, value| Ok(value))?
            }
            Value::Optional { ty, value } => {
                self.domain(ty)?;
                self.children(value.iter(), depth, siblings, |_, value| Ok(value.as_ref()))?
            }
            Value::ErrorResult { ty, value, .. } | Value::Newtype { ty, value } => {
                self.domain(ty)?;
                self.visit(value, depth + 1, siblings)?
            }
            Value::Type(ty) | Value::AbstractValue { ty, .. } => {
                self.domain(ty)?;
                0
            }
            Value::FunctionInstance { arguments, .. } => {
                for term in arguments {
                    self.term(term)?;
                }
                0
            }
            Value::RuntimeTraitMethod(method) => {
                for term in method.terms() {
                    self.term(term)?;
                }
                0
            }
            Value::IirCallable(handle) => {
                let before = self.expanded;
                self.domain(&handle.ty)?;
                for term in handle.key.arguments.iter() {
                    self.term(term)?;
                }
                self.shared += self.expanded - before;
                0
            }
            Value::String(text) | Value::Symbol(text) => {
                self.include(text.len())?;
                self.shared += text.len();
                0
            }
            Value::Integer(n) => {
                self.payload(n.bit_len().div_ceil(32))?;
                0
            }
            Value::Float(n) => {
                self.payload(
                    n.digits()
                        .len()
                        .saturating_add(n.exponent().bit_len().div_ceil(32)),
                )?;
                0
            }
            Value::BitVec(bits) => {
                self.payload(bits.words().len())?;
                0
            }
            Value::Closure(_)
            | Value::Code(_)
            | Value::Pointer(_)
            | Value::Storage(_)
            | Value::ComptimeReference(_) => return Err(ErrorKind::Unsupported),
            Value::Function(_)
            | Value::TypeParameter(_)
            | Value::Description(_)
            | Value::Float16(_)
            | Value::BFloat16(_)
            | Value::Float32(_)
            | Value::Float64(_)
            | Value::SignedInteger(_)
            | Value::UnsignedInteger(_)
            | Value::Unit
            | Value::Bool(_)
            | Value::Char(_) => 0,
        };
        self.reserve_children(siblings)?;
        Ok(height + 1)
    }
}

#[cfg(test)]
mod tests;
