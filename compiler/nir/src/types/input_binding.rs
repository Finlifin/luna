//! Checked specialization of one callable telescope. Domain refinement is
//! local to this telescope; ordinary substitution remains sort preserving.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionInputBinding {
    signature: Term,
    replacements: Vec<Term>,
    type_slots: BTreeSet<usize>,
}

pub(super) fn input_sort(parameter: &Parameter, expected: &Term) -> Sort {
    if parameter.sort == Sort::ComptimeValue && *expected == Term::Primitive(Primitive::Type) {
        Sort::Type
    } else {
        parameter.sort
    }
}

pub(super) fn replace_scope(
    term: &Term,
    arguments: &[Term],
    type_slots: &BTreeSet<usize>,
    remove: bool,
) -> Result<Term, TypeError> {
    term.rewrite(0, &mut |term, nesting| match term {
        Term::Bound { depth, index, sort } if *depth == nesting => {
            let value = arguments
                .get(*index)
                .ok_or(TypeError::MissingArgument(*index))?;
            let expected = if *sort == Sort::ComptimeValue && type_slots.contains(index) {
                Sort::Type
            } else {
                *sort
            };
            require_sort(value, expected)?;
            value.shift(nesting, 0)
        }
        Term::Bound { depth, index, sort } if remove && *depth > nesting => Ok(Term::Bound {
            depth: depth - 1,
            index: *index,
            sort: *sort,
        }),
        _ => Ok(term.clone()),
    })
}

/// Incremental binding of one callable telescope. Inputs are moved into the
/// substitution once; a failed push leaves the completed prefix unchanged.
#[derive(Debug)]
pub struct FunctionInputBuilder<'a> {
    original: &'a Term,
    parameters: &'a [Parameter],
    result: &'a Term,
    replacements: Vec<Term>,
    type_slots: BTreeSet<usize>,
    retained: Vec<Parameter>,
    retained_sorts: Vec<Sort>,
    supplied: Vec<bool>,
    next: Option<Term>,
}

impl<'a> FunctionInputBuilder<'a> {
    pub fn new(signature: &'a Term) -> Result<Self, TypeError> {
        let Term::Function { parameters, result } = signature.callable_signature() else {
            return Err(TypeError::InvalidFunctionSignature);
        };
        signature.check_scope(&[])?;
        Ok(Self {
            original: signature,
            parameters,
            result,
            replacements: Vec::with_capacity(parameters.len()),
            type_slots: BTreeSet::new(),
            retained: Vec::new(),
            retained_sorts: Vec::new(),
            supplied: Vec::with_capacity(parameters.len()),
            next: None,
        })
    }
    fn prepare_next(&mut self) -> Result<(), TypeError> {
        let index = self.replacements.len();
        let parameter = self.parameters.get(index).ok_or(TypeError::ArgumentCount {
            expected: self.parameters.len(),
            actual: index + 1,
        })?;
        if self.next.is_none() {
            self.next = Some(replace_scope(
                &parameter.ty,
                &self.replacements,
                &self.type_slots,
                false,
            )?);
        }
        Ok(())
    }
    /// The next declared domain after binding the completed prefix. Missing
    /// inputs retain their telescope indices rather than becoming dummy types.
    pub fn next_type(&mut self) -> Result<&Term, TypeError> {
        self.prepare_next()?;
        Ok(self.next.as_ref().unwrap())
    }
    pub fn next_type_bounded(
        &mut self,
        fuel: &mut usize,
    ) -> Result<&Term, crate::type_computations::Error> {
        use crate::type_computations::{Error, charge_frame_expansion};
        if self.next.is_none() {
            let index = self.replacements.len();
            let parameter =
                self.parameters
                    .get(index)
                    .ok_or(Error::Scope(TypeError::ArgumentCount {
                        expected: self.parameters.len(),
                        actual: index + 1,
                    }))?;
            charge_frame_expansion(&parameter.ty, &self.replacements, fuel)?;
        }
        self.next_type().map_err(Error::Scope)
    }

    /// Commit the receiving world's type view of the pending domain. The view
    /// must preserve its meaning; slot sort and scope are checked here.
    pub fn push_viewed(&mut self, input: Option<Term>, domain: Term) -> Result<(), TypeError> {
        if self.next.is_none() {
            return Err(TypeError::UnpreparedCallInput(self.replacements.len()));
        }
        self.push_domain(input, domain)?;
        self.next = None;
        Ok(())
    }

    pub fn push(&mut self, input: Option<Term>) -> Result<(), TypeError> {
        self.prepare_next()?;
        let ty = self.next.take().unwrap();
        self.push_domain(input, ty)
    }
    fn push_domain(&mut self, input: Option<Term>, ty: Term) -> Result<(), TypeError> {
        let index = self.replacements.len();
        let parameter = &self.parameters[index];
        require_sort(&ty, Sort::Type)?;
        ty.check_scope(std::slice::from_ref(&self.retained_sorts))?;
        let sort = input_sort(parameter, &ty);
        if let Some(input) = &input {
            input.check_scope(&[])?;
            require_sort(input, sort)?;
        }
        // All fallible checks precede mutation of the completed prefix.
        if sort != parameter.sort {
            self.type_slots.insert(index);
        }
        self.supplied.push(input.is_some());
        if let Some(input) = input {
            self.replacements.push(input);
        } else {
            self.replacements.push(Term::Bound {
                depth: 0,
                index: self.retained.len(),
                sort,
            });
            self.retained.push(Parameter { sort, ty });
            self.retained_sorts.push(sort);
        }
        Ok(())
    }
    pub fn finish_bounded(
        self,
        fuel: &mut usize,
    ) -> Result<FunctionInputBinding, crate::type_computations::Error> {
        crate::type_computations::charge_frame_expansion(self.result, &self.replacements, fuel)?;
        self.finish()
            .map_err(crate::type_computations::Error::Scope)
    }

    pub fn finish(self) -> Result<FunctionInputBinding, TypeError> {
        if self.replacements.len() != self.parameters.len() {
            return Err(TypeError::ArgumentCount {
                expected: self.parameters.len(),
                actual: self.replacements.len(),
            });
        }
        let specialized = Term::Function {
            parameters: self.retained,
            result: Box::new(replace_scope(
                self.result,
                &self.replacements,
                &self.type_slots,
                false,
            )?),
        };
        let signature = wrap_signature(self.original, specialized, &self.supplied);
        signature.check_scope(&[])?;
        Ok(FunctionInputBinding {
            signature,
            replacements: self.replacements,
            type_slots: self.type_slots,
        })
    }
}

fn wrap_signature(original: &Term, specialized: Term, supplied: &[bool]) -> Term {
    match original {
        Term::ContextualClosure { context, signature } => Term::ContextualClosure {
            context: context.clone(),
            signature: Box::new(wrap_signature(signature, specialized, supplied)),
        },
        Term::Closure {
            environment,
            signature,
        } => Term::Closure {
            environment: environment.clone(),
            signature: Box::new(wrap_signature(signature, specialized, supplied)),
        },
        Term::ComptimeFunction(inner) => {
            Term::ComptimeFunction(Box::new(wrap_signature(inner, specialized, supplied)))
        }
        Term::NormalFormFunction(inner, variadic) => Term::NormalFormFunction(
            Box::new(wrap_signature(inner, specialized, supplied)),
            variadic.and_then(|index| {
                (!supplied[index]).then(|| {
                    supplied[..index]
                        .iter()
                        .filter(|supplied| !**supplied)
                        .count()
                })
            }),
        ),
        Term::UnsafeFunction(inner) => {
            Term::UnsafeFunction(Box::new(wrap_signature(inner, specialized, supplied)))
        }
        _ => specialized,
    }
}

impl fmt::Display for FunctionInputBuilder<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(function-input-builder (original {}) (replacements",
            self.original
        )?;
        for term in &self.replacements {
            write!(f, " {term}")?;
        }
        f.write_str(") (type-slots")?;
        for slot in &self.type_slots {
            write!(f, " {slot}")?;
        }
        f.write_str(") (retained")?;
        for parameter in &self.retained {
            write!(f, " (parameter {} {})", parameter.sort, parameter.ty)?;
        }
        f.write_str(") (supplied")?;
        for supplied in &self.supplied {
            write!(f, " {supplied}")?;
        }
        f.write_str(") (next")?;
        if let Some(next) = &self.next {
            write!(f, " {next}")?;
        }
        f.write_str("))")
    }
}

impl FunctionInputBinding {
    pub(super) fn new(signature: &Term, inputs: &[Option<Term>]) -> Result<Self, TypeError> {
        check_input_count(signature, inputs.len())?;
        let mut builder = FunctionInputBuilder::new(signature)?;
        for input in inputs {
            builder.push(input.clone())?;
        }
        builder.finish()
    }
    pub(super) fn with_type_view<E: From<TypeError>>(
        signature: &Term,
        inputs: &[Option<Term>],
        mut view: impl FnMut(&Term, &[Parameter]) -> Result<Term, E>,
    ) -> Result<Self, E> {
        check_input_count(signature, inputs.len())?;
        let mut builder = FunctionInputBuilder::new(signature)?;
        for input in inputs {
            builder.prepare_next()?;
            let ty = builder.next.take().unwrap();
            let ty = view(&ty, &builder.retained)?;
            builder.push_domain(input.clone(), ty)?;
        }
        Ok(builder.finish()?)
    }

    pub fn signature(&self) -> &Term {
        &self.signature
    }
    pub fn into_signature(self) -> Term {
        self.signature
    }

    /// Borrow all stored terms for bounded structural observation. The binding
    /// also carries slot refinements, so these terms alone are not its identity.
    pub fn terms(&self) -> impl Iterator<Item = &Term> {
        std::iter::once(&self.signature).chain(&self.replacements)
    }

    /// Supply already admitted terms for retained inputs by original slot.
    /// The resulting binding still rewrites annotations of the original body.
    pub fn bind_retained_bounded(
        &self,
        inputs: std::collections::BTreeMap<usize, Term>,
        fuel: &mut usize,
    ) -> Result<Self, crate::type_computations::Error> {
        use crate::type_computations::Error;
        crate::copy_budget::charge_term_copy(&self.signature, fuel, 128)?;
        let mut builder = FunctionInputBuilder::new(&self.signature).map_err(Error::Scope)?;
        let mut retained: Vec<Option<Term>> = (0..builder.parameters.len()).map(|_| None).collect();
        for (original, value) in inputs {
            let Some(Term::Bound {
                depth: 0, index, ..
            }) = self.replacements.get(original)
            else {
                return Err(Error::Scope(TypeError::UnpreparedCallInput(original)));
            };
            crate::copy_budget::charge_term_copy(&value, fuel, 128)?;
            retained[*index] = Some(value);
        }
        for input in retained {
            builder.next_type_bounded(fuel)?;
            builder.push(input).map_err(Error::Scope)?;
        }
        let next = builder.finish_bounded(fuel)?;
        *fuel = fuel
            .checked_sub(self.replacements.len())
            .ok_or(Error::Budget)?;
        let mut remaining = next.replacements.into_iter().enumerate();
        let mut replacements = Vec::with_capacity(self.replacements.len());
        let mut type_slots = self.type_slots.clone();
        for (original, term) in self.replacements.iter().enumerate() {
            if let Term::Bound {
                depth: 0, index, ..
            } = term
            {
                // Each retained slot occurs once, in telescope order. Move its
                // completed term without copying a potentially large payload.
                let (position, replacement) = remaining
                    .next()
                    .ok_or(Error::Scope(TypeError::MissingArgument(*index)))?;
                debug_assert_eq!(position, *index);
                if next.type_slots.contains(index) {
                    type_slots.insert(original);
                }
                replacements.push(replacement);
            } else {
                replacements.push(crate::copy_budget::clone_term(term, fuel)?);
            }
        }
        Ok(Self {
            signature: next.signature,
            replacements,
            type_slots,
        })
    }

    pub fn rebind_bounded(
        &self,
        term: &Term,
        fuel: &mut usize,
    ) -> Result<Term, crate::type_computations::Error> {
        crate::type_computations::charge_frame_expansion(term, &self.replacements, fuel)?;
        self.rebind(term)
            .map_err(crate::type_computations::Error::Scope)
    }

    /// Rebind annotations from the original function frame. Only a value slot
    /// whose instantiated type is exactly Type may change semantic domain.
    pub fn rebind(&self, term: &Term) -> Result<Term, TypeError> {
        replace_scope(term, &self.replacements, &self.type_slots, false)
    }
}

impl fmt::Display for FunctionInputBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(function-input-binding (signature {}) (replacements",
            self.signature
        )?;
        for replacement in &self.replacements {
            write!(f, " {replacement}")?;
        }
        f.write_str(") (type-slots")?;
        for slot in &self.type_slots {
            write!(f, " {slot}")?;
        }
        f.write_str("))")
    }
}

fn check_input_count(signature: &Term, count: usize) -> Result<(), TypeError> {
    let Term::Function { parameters, .. } = signature.callable_signature() else {
        return Err(TypeError::InvalidFunctionSignature);
    };
    if parameters.len() != count {
        return Err(TypeError::ArgumentCount {
            expected: parameters.len(),
            actual: count,
        });
    }
    Ok(())
}

/// A checked substitution from a callee telescope into its caller's scope.
/// Missing inputs remain unavailable; this does not manufacture residual binders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionCallBinding {
    arguments: Vec<Option<Term>>,
    type_slots: BTreeSet<usize>,
    // The last frame belongs to the callee. Earlier frames belong to the caller.
    frames: Vec<Vec<Sort>>,
}

/// A call keeps one checked telescope while producers complete in dependency
/// order. Only admitted observations enter substitution; a failed delivery
/// leaves the slot open for the original consumer.
pub struct FunctionCallPreparation<S> {
    signature: S,
    pending: std::collections::BTreeMap<usize, Sort>,
    binding: FunctionCallBinding,
}

impl<S: std::borrow::Borrow<Term>> FunctionCallPreparation<S> {
    pub fn new(signature: S, mut caller_frames: Vec<Vec<Sort>>) -> Result<Self, TypeError> {
        let Term::Function { parameters, .. } = signature.borrow().callable_signature() else {
            return Err(TypeError::InvalidFunctionSignature);
        };
        signature.borrow().check_scope(&caller_frames)?;
        caller_frames.push(parameters.iter().map(|p| p.sort).collect());
        let count = parameters.len();
        Ok(Self {
            signature,
            pending: Default::default(),
            binding: FunctionCallBinding {
                frames: caller_frames,
                arguments: vec![None; count],
                type_slots: BTreeSet::new(),
            },
        })
    }

    pub fn parameters(&self) -> &[Parameter] {
        let Term::Function { parameters, .. } = self.signature.borrow().callable_signature() else {
            unreachable!("validated callable telescope")
        };
        parameters
    }

    pub fn binding(&self) -> &FunctionCallBinding {
        &self.binding
    }

    pub fn finish(self) -> Result<FunctionCallBinding, TypeError> {
        self.finish_with_signature().map(|(_, binding)| binding)
    }

    pub fn finish_with_signature(self) -> Result<(S, FunctionCallBinding), TypeError> {
        if let Some((&slot, _)) = self.pending.first_key_value() {
            return Err(TypeError::MissingArgument(slot));
        }
        Ok((self.signature, self.binding))
    }

    /// Return the domain to check the source in. The call retains its proven
    /// sort until this particular producer returns, including across defaults.
    pub fn prepare<E>(
        &mut self,
        index: usize,
        view: impl FnOnce(&Term) -> Result<Term, E>,
    ) -> Result<Term, CallError<E>> {
        self.prepare_using(index, view, FunctionCallBinding::substitute)
    }

    pub fn prepare_bounded<E>(
        &mut self,
        index: usize,
        fuel: &mut usize,
        view: impl FnOnce(&Term) -> Result<Term, E>,
    ) -> Result<Term, CallError<E>> {
        self.prepare_using(index, view, |binding, ty| {
            binding.substitute_bounded(ty, fuel)
        })
    }

    fn prepare_using<E>(
        &mut self,
        index: usize,
        view: impl FnOnce(&Term) -> Result<Term, E>,
        substitute: impl FnOnce(&FunctionCallBinding, &Term) -> Result<Term, TypeError>,
    ) -> Result<Term, CallError<E>> {
        let parameter = self
            .parameters()
            .get(index)
            .ok_or(CallError::Shape(TypeError::MissingArgument(index)))?;
        if self.binding.arguments[index].is_some() || self.pending.contains_key(&index) {
            return Err(CallError::Shape(TypeError::DuplicateCallInput(index)));
        }
        let caller = self.binding.caller_frames();
        let domain = substitute(&self.binding, &parameter.ty).map_err(CallError::Shape)?;
        let domain = view(&domain).map_err(|error| CallError::Contract {
            parameter: index,
            error,
        })?;
        require_sort(&domain, Sort::Type).map_err(CallError::Shape)?;
        domain.check_scope(caller).map_err(CallError::Shape)?;
        self.pending.insert(index, input_sort(parameter, &domain));
        Ok(domain)
    }

    pub fn supply(&mut self, index: usize, input: Term) -> Result<(), TypeError> {
        let sort = *self
            .pending
            .get(&index)
            .ok_or(TypeError::UnpreparedCallInput(index))?;
        input.check_scope(self.binding.caller_frames())?;
        require_sort(&input, sort)?;
        self.pending.remove(&index);
        if sort != self.parameters()[index].sort {
            self.binding.type_slots.insert(index);
        }
        self.binding.arguments[index] = Some(input);
        Ok(())
    }
}

impl<S> fmt::Display for FunctionCallPreparation<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(function-call-preparation {} (pending", self.binding)?;
        for (slot, sort) in &self.pending {
            write!(f, " (input {slot} {sort})")?;
        }
        f.write_str("))")
    }
}

impl FunctionCallBinding {
    pub fn new<E>(
        signature: &Term,
        inputs: &[Option<Term>],
        caller_frames: &[Vec<Sort>],
        mut view: impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<Self, CallError<E>> {
        Self::from_observations(
            signature,
            inputs,
            caller_frames.to_vec(),
            |call, index, input| {
                call.prepare(index, &mut view)?;
                call.supply(index, input.clone()).map_err(CallError::Shape)
            },
        )
    }

    pub fn new_bounded<E>(
        signature: &Term,
        inputs: &[Option<Term>],
        caller_frames: Vec<Vec<Sort>>,
        fuel: &mut usize,
        mut view: impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<Self, CallError<E>> {
        crate::type_computations::charge_term(signature, fuel)
            .map_err(|_| CallError::Shape(TypeError::ConstructionBudget))?;
        Self::from_observations(signature, inputs, caller_frames, |call, index, input| {
            call.prepare_bounded(index, fuel, &mut view)?;
            let input = crate::copy_budget::clone_term(input, fuel)
                .map_err(|_| CallError::Shape(TypeError::ConstructionBudget))?;
            call.supply(index, input).map_err(CallError::Shape)
        })
    }

    fn from_observations<E>(
        signature: &Term,
        inputs: &[Option<Term>],
        caller_frames: Vec<Vec<Sort>>,
        mut supply: impl FnMut(
            &mut FunctionCallPreparation<&Term>,
            usize,
            &Term,
        ) -> Result<(), CallError<E>>,
    ) -> Result<Self, CallError<E>> {
        let Term::Function { parameters, .. } = signature.callable_signature() else {
            return Err(CallError::NotFunction);
        };
        if inputs.len() != parameters.len() {
            return Err(CallError::Shape(TypeError::ArgumentCount {
                expected: parameters.len(),
                actual: inputs.len(),
            }));
        }
        let mut call =
            FunctionCallPreparation::new(signature, caller_frames).map_err(CallError::Shape)?;
        for (index, input) in inputs.iter().enumerate() {
            if let Some(input) = input {
                supply(&mut call, index, input)?;
            }
        }
        call.finish().map_err(CallError::Shape)
    }

    fn caller_frames(&self) -> &[Vec<Sort>] {
        &self.frames[..self.frames.len() - 1]
    }

    pub fn observations(&self) -> &[Option<Term>] {
        &self.arguments
    }

    /// Charge the actual expansion before copying it. An unobserved or unrelated
    /// slot costs nothing unless the template reaches it at this binder depth.
    pub fn substitute_bounded(&self, term: &Term, fuel: &mut usize) -> Result<Term, TypeError> {
        crate::type_computations::charge_frame_slots(
            term,
            |index| self.arguments.get(index)?.as_ref(),
            fuel,
        )
        .map_err(|_| TypeError::ConstructionBudget)?;
        self.substitute(term)
    }

    pub fn substitute(&self, term: &Term) -> Result<Term, TypeError> {
        term.check_scope(&self.frames)?;
        let result = term.rewrite(0, &mut |term, nesting| match term {
            Term::Bound { depth, index, sort } if *depth == nesting => {
                let argument = self
                    .arguments
                    .get(*index)
                    .and_then(Option::as_ref)
                    .ok_or(TypeError::MissingArgument(*index))?;
                let expected = if *sort == Sort::ComptimeValue && self.type_slots.contains(index) {
                    Sort::Type
                } else {
                    *sort
                };
                require_sort(argument, expected)?;
                argument.shift(nesting, 0)
            }
            Term::Bound { depth, index, sort } if *depth > nesting => Ok(Term::Bound {
                depth: depth - 1,
                index: *index,
                sort: *sort,
            }),
            _ => Ok(term.clone()),
        })?;
        result.check_scope(self.caller_frames())?;
        Ok(result)
    }
}

impl fmt::Display for FunctionCallBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(function-call-binding (arguments")?;
        for input in &self.arguments {
            match input {
                Some(input) => write!(f, " (some {input})")?,
                None => f.write_str(" (missing)")?,
            }
        }
        f.write_str(") (caller-frames")?;
        for frame in self.caller_frames() {
            f.write_str(" (frame")?;
            for sort in frame {
                write!(f, " {sort}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (callee-sorts")?;
        for sort in self.frames.last().unwrap() {
            write!(f, " {sort}")?;
        }
        f.write_str(") (type-slots")?;
        for slot in &self.type_slots {
            write!(f, " {slot}")?;
        }
        f.write_str("))")
    }
}
