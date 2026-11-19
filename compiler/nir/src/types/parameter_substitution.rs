//! Simultaneous structural substitution for explicitly sorted declaration
//! inputs. Checking their sorts and scope grants no value-domain evidence.
use super::*;
use crate::type_computations::{self, Error};

pub(crate) struct ParameterSubstitution<'a, 'scope> {
    bindings: BTreeMap<DeclarationId, (Sort, &'a Term)>,
    // Fragment owners validate the surrounding telescope after assembly.
    result_frames: Option<&'scope [Vec<Sort>]>,
}

impl<'a, 'scope> ParameterSubstitution<'a, 'scope> {
    pub(crate) fn for_fragments(
        parameters: &[DeclarationParameter],
        arguments: &'a [Term],
    ) -> Result<Self, TypeError> {
        Self::from_slices(parameters, arguments, None)
    }

    pub(crate) fn in_scope(
        parameters: &[DeclarationParameter],
        arguments: &'a [Term],
        frames: &'scope [Vec<Sort>],
    ) -> Result<Self, TypeError> {
        Self::from_slices(parameters, arguments, Some(frames))
    }

    fn from_slices(
        parameters: &[DeclarationParameter],
        arguments: &'a [Term],
        result_frames: Option<&'scope [Vec<Sort>]>,
    ) -> Result<Self, TypeError> {
        if parameters.len() != arguments.len() {
            return Err(TypeError::ArgumentCount {
                expected: parameters.len(),
                actual: arguments.len(),
            });
        }
        let mut substitution = Self {
            bindings: BTreeMap::new(),
            result_frames,
        };
        for (&parameter, argument) in parameters.iter().zip(arguments) {
            substitution.bind(parameter, argument)?;
        }
        Ok(substitution)
    }

    fn bind(&mut self, parameter: DeclarationParameter, value: &'a Term) -> Result<(), TypeError> {
        value.check_scope(self.result_frames.unwrap_or(&[]))?;
        require_sort(value, parameter.sort)?;
        if self
            .bindings
            .insert(parameter.declaration, (parameter.sort, value))
            .is_some()
        {
            return Err(TypeError::DuplicateDeclarationParameter(
                parameter.declaration,
            ));
        }
        Ok(())
    }

    pub(crate) fn term(&self, root: &Term) -> Result<Term, TypeError> {
        let result = root.replace_declarations(&|id| self.bindings.get(&id).copied())?;
        if let Some(frames) = self.result_frames {
            result.check_scope(frames)?;
        }
        Ok(result)
    }

    pub(crate) fn instantiate_bounded(
        root: &Term,
        bindings: impl IntoIterator<Item = (DeclarationParameter, &'a Term)>,
        frames: &'scope [Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Term, Error> {
        let mut substitution = Self {
            bindings: BTreeMap::new(),
            result_frames: Some(frames),
        };
        for (parameter, value) in bindings {
            *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
            type_computations::charge_term(value, fuel)?;
            substitution.bind(parameter, value).map_err(Error::Scope)?;
        }
        type_computations::visit_expansion(
            root,
            fuel,
            128,
            &|id| substitution.bindings.get(&id).map(|&(_, value)| value),
            &mut |_, _| {},
        )?;
        substitution.term(root).map_err(Error::Scope)
    }
}
