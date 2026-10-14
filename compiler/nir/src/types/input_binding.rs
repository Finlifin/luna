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

impl FunctionInputBinding {
    pub(super) fn new(signature: &Term, inputs: &[Option<Term>]) -> Result<Self, TypeError> {
        Self::with_type_view(signature, inputs, |ty, _| Ok::<_, TypeError>(ty.clone()))
    }

    pub(super) fn with_type_view<E: From<TypeError>>(
        signature: &Term,
        inputs: &[Option<Term>],
        mut view: impl FnMut(&Term, &[Parameter]) -> Result<Term, E>,
    ) -> Result<Self, E> {
        let Term::Function { parameters, result } = signature.callable_signature() else {
            return Err(TypeError::InvalidFunctionSignature.into());
        };
        if inputs.len() != parameters.len() {
            return Err(TypeError::ArgumentCount {
                expected: parameters.len(),
                actual: inputs.len(),
            }.into());
        }
        signature.check_scope(&[])?;
        let mut replacements = Vec::with_capacity(inputs.len());
        let mut type_slots = BTreeSet::new();
        let mut retained = Vec::new();
        for (index, (parameter, input)) in parameters.iter().zip(inputs).enumerate() {
            let ty = replace_scope(&parameter.ty, &replacements, &type_slots, false)?;
            let ty = view(&ty, &retained)?;
            require_sort(&ty, Sort::Type)?;
            ty.check_scope(&[retained.iter().map(|p| p.sort).collect()])?;
            let sort = input_sort(parameter, &ty);
            if sort != parameter.sort {
                type_slots.insert(index);
            }
            if let Some(input) = input {
                input.check_scope(&[])?;
                require_sort(input, sort)?;
                replacements.push(input.clone());
            } else {
                replacements.push(Term::Bound {
                    depth: 0,
                    index: retained.len(),
                    sort,
                });
                retained.push(Parameter { sort, ty });
            }
        }
        let specialized = Term::Function {
            parameters: retained,
            result: Box::new(replace_scope(result, &replacements, &type_slots, false)?),
        };
        fn wrap(original: &Term, specialized: Term) -> Term {
            match original {
                Term::Closure {
                    environment,
                    signature,
                } => Term::Closure {
                    environment: environment.clone(),
                    signature: Box::new(wrap(signature, specialized)),
                },
                Term::ComptimeFunction(inner) => {
                    Term::ComptimeFunction(Box::new(wrap(inner, specialized)))
                }
                _ => specialized,
            }
        }
        let signature = wrap(signature, specialized);
        signature.check_scope(&[])?;
        Ok(Self {
            signature,
            replacements,
            type_slots,
        })
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
                expected: parameters.len(), actual: inputs.len(),
            }));
        }
        signature.check_scope(caller_frames).map_err(CallError::Shape)?;
        let mut binding = Self {
            arguments: inputs.to_vec(), type_slots: BTreeSet::new(),
            caller_frames: caller_frames.to_vec(),
            callee_sorts: parameters.iter().map(|p| p.sort).collect(),
        };
        for (index, (parameter, input)) in parameters.iter().zip(inputs).enumerate() {
            let Some(input) = input else { continue };
            input.check_scope(caller_frames).map_err(CallError::Shape)?;
            let domain = binding.substitute(&parameter.ty).map_err(CallError::Shape)?;
            let domain = view(&domain).map_err(|error| CallError::Contract { parameter: index, error })?;
            require_sort(&domain, Sort::Type).map_err(CallError::Shape)?;
            domain.check_scope(caller_frames).map_err(CallError::Shape)?;
            let sort = input_sort(parameter, &domain);
            require_sort(input, sort).map_err(CallError::Shape)?;
            if sort != parameter.sort { binding.type_slots.insert(index); }
        }
        Ok(binding)
    }

    pub fn substitute(&self, term: &Term) -> Result<Term, TypeError> {
        let mut source_frames = self.caller_frames.clone();
        source_frames.push(self.callee_sorts.clone());
        term.check_scope(&source_frames)?;
        let result = term.rewrite(0, &mut |term, nesting| match term {
            Term::Bound { depth, index, sort } if *depth == nesting => {
                let argument = self.arguments.get(*index).and_then(Option::as_ref)
                    .ok_or(TypeError::MissingArgument(*index))?;
                let expected = if *sort == Sort::ComptimeValue && self.type_slots.contains(index) {
                    Sort::Type
                } else { *sort };
                require_sort(argument, expected)?;
                argument.shift(nesting, 0)
            }
            Term::Bound { depth, index, sort } if *depth > nesting => Ok(Term::Bound {
                depth: depth - 1, index: *index, sort: *sort,
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
            for sort in frame { write!(f, " {sort}")?; }
            f.write_str(")")?;
        }
        f.write_str(") (callee-sorts")?;
        for sort in &self.callee_sorts { write!(f, " {sort}")?; }
        f.write_str(") (type-slots")?;
        for slot in &self.type_slots { write!(f, " {slot}")?; }
        f.write_str("))")
    }
}
