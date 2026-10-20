//! Saved signature computations, separate from immutable function-type identity.
//! A deferred annotation is not an opaque type head or a unification variable.
use crate::type_computations::Invocation;
use crate::{Parameter, Primitive, Sort, Term, TypeError};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Annotation {
    Known(Term),
    Computed(usize),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub sort: Sort,
    pub annotation: Annotation,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    inputs: Vec<Input>,
    result: Annotation,
    computations: Vec<Invocation>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Budget,
    Scope(TypeError),
    MissingComputation(usize),
    UnusedComputation(usize),
    InputFrame(usize),
    ForwardInput { computation: usize, input: usize },
    TypeParameterDomain(usize),
    ResultCount,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(deferred-signature-error {:?})", format!("{self:?}"))
    }
}
impl std::error::Error for Error {}

impl Signature {
    pub fn new(
        inputs: Vec<Input>,
        result: Annotation,
        computations: Vec<Invocation>,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        *fuel = fuel
            .checked_sub(
                inputs
                    .len()
                    .saturating_add(computations.len())
                    .saturating_add(1),
            )
            .ok_or(Error::Budget)?;
        let scope = inputs.iter().map(|input| input.sort).collect::<Vec<_>>();
        for (index, computation) in computations.iter().enumerate() {
            *fuel = fuel.checked_sub(scope.len()).ok_or(Error::Budget)?;
            if computation.scope() != scope {
                return Err(Error::InputFrame(index));
            }
        }
        let mut used = vec![false; computations.len()];
        let mut frames = vec![Vec::with_capacity(scope.len())];
        for (slot, annotation) in inputs
            .iter()
            .map(|input| &input.annotation)
            .chain(std::iter::once(&result))
            .enumerate()
        {
            match annotation {
                Annotation::Known(ty) => {
                    crate::type_computations::bounded(ty, fuel).map_err(|_| Error::Budget)?;
                    if ty.sort() != Sort::Type {
                        return Err(Error::Scope(TypeError::WrongSort {
                            expected: Sort::Type,
                            actual: ty.sort(),
                        }));
                    }
                    ty.check_scope(&frames).map_err(Error::Scope)?;
                }
                Annotation::Computed(index) => {
                    let computation = computations
                        .get(*index)
                        .ok_or(Error::MissingComputation(*index))?;
                    used[*index] = true;
                    *fuel = fuel
                        .checked_sub(computation.dependencies().len())
                        .ok_or(Error::Budget)?;
                    if let Some(input) = computation
                        .dependencies()
                        .iter()
                        .find(|input| **input >= slot)
                    {
                        return Err(Error::ForwardInput {
                            computation: *index,
                            input: *input,
                        });
                    }
                }
            }
            if slot < inputs.len()
                && inputs[slot].sort == Sort::Type
                && *annotation != Annotation::Known(Term::Primitive(Primitive::Type))
            {
                return Err(Error::TypeParameterDomain(slot));
            }
            if slot < scope.len() {
                frames[0].push(scope[slot]);
            }
        }
        if let Some(index) = used.iter().position(|used| !used) {
            return Err(Error::UnusedComputation(index));
        }
        Ok(Self {
            inputs,
            result,
            computations,
        })
    }
    pub fn inputs(&self) -> &[Input] {
        &self.inputs
    }
    pub fn result(&self) -> &Annotation {
        &self.result
    }
    pub fn computations(&self) -> &[Invocation] {
        &self.computations
    }

    /// Replace completed annotations while retaining the original input frame.
    /// Staged-input substitution and runtime signature projection remain the
    /// existing function-input preparation's responsibility.
    pub fn resolve(&self, results: &[&Term], fuel: &mut usize) -> Result<Term, Error> {
        if results.len() != self.computations.len() {
            return Err(Error::ResultCount);
        }
        *fuel = fuel
            .checked_sub(
                self.inputs
                    .len()
                    .saturating_add(results.len())
                    .saturating_add(1),
            )
            .ok_or(Error::Budget)?;
        for result in results {
            crate::type_computations::bounded(result, fuel).map_err(|_| Error::Budget)?;
            if result.sort() != Sort::Type {
                return Err(Error::Scope(TypeError::WrongSort {
                    expected: Sort::Type,
                    actual: result.sort(),
                }));
            }
            result.check_scope(&[]).map_err(Error::Scope)?;
        }
        let mut annotation = |annotation: &Annotation| -> Result<Term, Error> {
            let term = match annotation {
                Annotation::Known(ty) => ty,
                Annotation::Computed(index) => results[*index],
            };
            crate::type_computations::bounded(term, fuel).map_err(|_| Error::Budget)?;
            Ok(term.clone())
        };
        let parameters = self
            .inputs
            .iter()
            .map(|input| {
                Ok(Parameter {
                    sort: input.sort,
                    ty: annotation(&input.annotation)?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let result = annotation(&self.result)?;
        let signature = Term::Function {
            parameters,
            result: Box::new(result),
        };
        signature.check_scope(&[]).map_err(Error::Scope)?;
        Ok(signature)
    }
}
impl fmt::Display for Annotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Known(ty) => write!(f, "(known {ty})"),
            Self::Computed(index) => write!(f, "(computed {index})"),
        }
    }
}
impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(input {} {})", self.sort, self.annotation)
    }
}
impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(deferred-signature (inputs")?;
        for input in &self.inputs {
            write!(f, " {input}")?;
        }
        write!(f, ") (result {}) (computations", self.result)?;
        for computation in &self.computations {
            write!(f, " {computation}")?;
        }
        write!(f, "))")
    }
}
