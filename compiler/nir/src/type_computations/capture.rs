//! Lexical reads belong to a checked body. Type-slot dependencies are
//! selected after rebinding into the caller's declaration frame.
use super::*;

/// A compact entry plus its input positions in the original declaration frame.
/// This stable mapping is used to supply each instance, not a mutable waiter.
#[derive(Clone, Debug, PartialEq)]
pub struct CapturedEntry {
    pub function: crate::program::Function,
    pub slots: Vec<usize>,
    source_scope: Vec<Sort>,
}
impl CapturedEntry {
    pub fn source_scope(&self) -> &[Sort] {
        &self.source_scope
    }

    /// Save the call in the source declaration's frame, even though the owned
    /// entry uses a compact frame. Instantiation can then supply its ordinary
    /// input telescope directly; pruned runtime slots never become waiters.
    pub fn invocation(&self, fuel: &mut usize) -> Result<Invocation, Error> {
        *fuel = fuel
            .checked_sub(self.source_scope.len())
            .ok_or(Error::Budget)?;
        let compact = Invocation::from_declaration(&self.function.interface, fuel)?;
        if compact.scope().len() != self.slots.len() {
            return Err(Error::InvalidCall(self.function.source));
        }
        let mut seen = BTreeSet::new();
        let mut mapping = Vec::with_capacity(self.slots.len());
        for (index, slot) in self.slots.iter().copied().enumerate() {
            let sort = *self
                .source_scope
                .get(slot)
                .ok_or(Error::Scope(TypeError::MissingArgument(slot)))?;
            if !seen.insert(slot) {
                return Err(Error::InvalidCall(self.function.source));
            }
            if sort != compact.scope()[index] {
                return Err(Error::Scope(TypeError::WrongSort {
                    expected: compact.scope()[index],
                    actual: sort,
                }));
            }
            mapping.push(Some((slot, sort)));
        }
        let inputs = compact
            .inputs()
            .iter()
            .map(|input| {
                bounded(&input.ty, fuel)?;
                bounded(&input.value, fuel)?;
                Ok(Input {
                    ty: input.ty.relocate_slots(&mapping).map_err(Error::Scope)?,
                    value: input.value.relocate_slots(&mapping).map_err(Error::Scope)?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Invocation::new(compact.target(), self.source_scope.clone(), inputs, fuel)
    }
}
impl fmt::Display for CapturedEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(captured-comptime-entry (slots")?;
        for slot in &self.slots {
            write!(f, " {slot}")?;
        }
        write!(f, ") (source-scope")?;
        for sort in &self.source_scope {
            write!(f, " {sort}")?;
        }
        write!(f, ") (function {}))", self.function)
    }
}
/// Discover free value inputs and transitive annotation dependencies, retaining
/// their original order. Expr annotations use the enclosing ordinary input
/// frame; nested callable signatures introduce their own Term binder normally.
pub fn capture_entry(
    declaration: DeclarationId,
    name: String,
    body: Expr,
    available: &[(LocalId, crate::Parameter)],
    fuel: &mut usize,
) -> Result<CapturedEntry, Error> {
    capture_entry_domains(
        declaration,
        name,
        body,
        available
            .iter()
            .map(|(id, parameter)| (*id, parameter.sort, Some(&parameter.ty))),
        fuel,
    )
}

/// Capture a source telescope with unresolved domains. Such slots keep their
/// positions and phase but cannot be captured until their domains are known.
/// Runtime inputs remain forbidden even when their domains are known.
pub fn capture_sparse_entry(
    declaration: DeclarationId,
    name: String,
    body: Expr,
    available: &[(LocalId, Sort, Option<Term>)],
    fuel: &mut usize,
) -> Result<CapturedEntry, Error> {
    capture_entry_domains(
        declaration,
        name,
        body,
        available
            .iter()
            .map(|(id, sort, domain)| (*id, *sort, domain.as_ref())),
        fuel,
    )
}

/// Capture from borrowed source domains. Only the selected domains and the
/// projected body become part of the returned entry.
pub fn capture_entry_domains<'a>(
    declaration: DeclarationId,
    name: String,
    body: Expr,
    domains: impl ExactSizeIterator<Item = (LocalId, Sort, Option<&'a Term>)>,
    fuel: &mut usize,
) -> Result<CapturedEntry, Error> {
    CaptureBody::analyze(body, fuel)?.capture(declaration, name, domains, fuel)
}

/// Own the checked code together with its free lexical reads. Source checking
/// can use those reads to supply the entry telescope. Rebinding type slots
/// preserves LocalIds, but type-slot dependencies must be computed afterwards.
pub struct CaptureBody {
    body: Expr,
    reads: BTreeSet<LocalId>,
}

impl CaptureBody {
    pub fn analyze(body: Expr, fuel: &mut usize) -> Result<Self, crate::free_bindings::Error> {
        let reads = crate::free_bindings::collect(&body, fuel)?;
        Ok(Self { body, reads })
    }

    pub fn reads(&self) -> &BTreeSet<LocalId> {
        &self.reads
    }

    pub fn rewrite_types<E>(
        mut self,
        transform: &mut impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<Self, E> {
        self.body.rewrite_types(transform)?;
        Ok(self)
    }

    pub fn capture<'a>(
        self,
        declaration: DeclarationId,
        name: String,
        domains: impl ExactSizeIterator<Item = (LocalId, Sort, Option<&'a Term>)>,
        fuel: &mut usize,
    ) -> Result<CapturedEntry, Error> {
        let Self { mut body, reads } = self;
        *fuel = fuel.checked_sub(domains.len()).ok_or(Error::Budget)?;
        let available: Vec<_> = domains.collect();
        let mut positions = BTreeMap::new();
        for (slot, (id, _, _)) in available.iter().enumerate() {
            if positions.insert(*id, slot).is_some() {
                return Err(Error::DuplicateBinding(*id));
            }
        }
        let frames = [available
            .iter()
            .map(|(_, sort, _)| *sort)
            .collect::<Vec<_>>()];
        let body_slots = crate::copy_budget::expression_capture_slots(&body, &frames, fuel)?;
        let mut needed = body_slots.clone();
        for id in reads {
            needed.insert(*positions.get(&id).ok_or(Error::MissingCapture(id))?);
        }
        let mut work = needed.iter().copied().collect::<Vec<_>>();
        while let Some(slot) = work.pop() {
            let (_, sort, domain) = available
                .get(slot)
                .ok_or(Error::Scope(TypeError::MissingArgument(slot)))?;
            if *sort == Sort::Runtime {
                return Err(Error::RuntimeInput(slot));
            }
            let domain = domain
                .as_ref()
                .ok_or(Error::Scope(TypeError::MissingArgument(slot)))?;
            for dependency in crate::copy_budget::domain_capture_slots(domain, &frames, fuel)? {
                if needed.insert(dependency) {
                    work.push(dependency);
                }
            }
        }
        let slots = needed.into_iter().collect::<Vec<_>>();
        let mut mapping = vec![None; available.len()];
        for (new, old) in slots.iter().copied().enumerate() {
            mapping[old] = Some((new, available[old].1));
        }
        if slots
            .iter()
            .enumerate()
            .any(|(new, old)| new != *old && body_slots.contains(old))
        {
            // Renumbering bound references preserves every expression and pattern
            // category. No value has been supplied by frame compaction.
            let mut steps = *fuel as u64;
            let charged = crate::copy_budget::charge_type_rewrite(&body, &mut steps);
            *fuel = steps as usize;
            charged?;
            body.relocate_slots(&mapping).map_err(Error::Scope)?;
        }
        let inputs = slots
            .iter()
            .map(|slot| {
                let (id, sort, domain) = &available[*slot];
                Ok((
                    *id,
                    crate::Parameter {
                        sort: *sort,
                        ty: domain
                            .as_ref()
                            .ok_or(Error::Scope(TypeError::MissingArgument(*slot)))?
                            .relocate_slots(&mapping)
                            .map_err(Error::Scope)?,
                    },
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        // The capture closure contains every free local, including reads from
        // operand preparation. Frame relocation cannot introduce local identities.
        let function = entry_with_bound_locals(declaration, name, body, inputs, fuel)?;
        bounded(&function.signature, fuel)?;
        function.signature.check_scope(&[]).map_err(Error::Scope)?;
        Ok(CapturedEntry {
            function,
            slots,
            source_scope: frames.into_iter().next().unwrap(),
        })
    }
}

impl fmt::Display for CaptureBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(capture-body (reads")?;
        for binding in &self.reads {
            write!(f, " {binding}")?;
        }
        write!(f, ") {})", self.body)
    }
}
