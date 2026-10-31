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
    pub fn push(&mut self, input: Option<Term>) -> Result<(), TypeError> {
        self.prepare_next()?;
        let ty = self.next.take().unwrap();
        self.push_domain(input, ty)
    }
    fn push_domain(&mut self, input: Option<Term>, ty: Term) -> Result<(), TypeError> {
        let index = self.replacements.len();
        let parameter = &self.parameters[index];
        require_sort(&ty, Sort::Type)?;
        ty.check_scope(&[self.retained.iter().map(|p| p.sort).collect()])?;
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
        }
        Ok(())
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
    caller_frames: Vec<Vec<Sort>>,
    callee_sorts: Vec<Sort>,
}

impl FunctionCallBinding {
    pub fn new<E>(
        signature: &Term,
        inputs: &[Option<Term>],
        caller_frames: &[Vec<Sort>],
        mut view: impl FnMut(&Term) -> Result<Term, E>,
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
        signature
            .check_scope(caller_frames)
            .map_err(CallError::Shape)?;
        let mut binding = Self {
            arguments: inputs.to_vec(),
            type_slots: BTreeSet::new(),
            caller_frames: caller_frames.to_vec(),
            callee_sorts: parameters.iter().map(|p| p.sort).collect(),
        };
        for (index, (parameter, input)) in parameters.iter().zip(inputs).enumerate() {
            let Some(input) = input else { continue };
            input.check_scope(caller_frames).map_err(CallError::Shape)?;
            let domain = binding
                .substitute(&parameter.ty)
                .map_err(CallError::Shape)?;
            let domain = view(&domain).map_err(|error| CallError::Contract {
                parameter: index,
                error,
            })?;
            require_sort(&domain, Sort::Type).map_err(CallError::Shape)?;
            domain
                .check_scope(caller_frames)
                .map_err(CallError::Shape)?;
            let sort = input_sort(parameter, &domain);
            require_sort(input, sort).map_err(CallError::Shape)?;
            if sort != parameter.sort {
                binding.type_slots.insert(index);
            }
        }
        Ok(binding)
    }

    pub fn substitute(&self, term: &Term) -> Result<Term, TypeError> {
        let mut source_frames = self.caller_frames.clone();
        source_frames.push(self.callee_sorts.clone());
        term.check_scope(&source_frames)?;
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
        result.check_scope(&self.caller_frames)?;
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
        for frame in &self.caller_frames {
            f.write_str(" (frame")?;
            for sort in frame {
                write!(f, " {sort}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (callee-sorts")?;
        for sort in &self.callee_sorts {
            write!(f, " {sort}")?;
        }
        f.write_str(") (type-slots")?;
        for slot in &self.type_slots {
            write!(f, " {slot}")?;
        }
        f.write_str("))")
    }
}
