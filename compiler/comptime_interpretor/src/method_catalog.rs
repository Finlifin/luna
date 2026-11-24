//! Immutable method binders with contract references resolved in the source slice.
use super::*;

pub(super) struct Entry {
    pub(super) parameters: Arc<[DeclarationId]>,
    pub(super) interface: usize,
    pub(super) member: usize,
}

pub(super) type Entries = BTreeMap<(DeclarationId, DeclarationId), Entry>;

#[derive(Default)]
pub(super) struct Contracts<'a> {
    pub(super) traits: &'a [nir::program::Trait],
    pub(super) entries: Arc<Entries>,
}

impl<'a> Contracts<'a> {
    pub(super) fn get(
        &self,
        key: &(DeclarationId, DeclarationId),
    ) -> Option<(&Arc<[DeclarationId]>, &'a nir::program::MethodContract)> {
        let entry = self.entries.get(key)?;
        Some((
            &entry.parameters,
            &self.traits[entry.interface].methods[entry.member],
        ))
    }
}

/// Selected public declaration inputs and the ordinary frame of its code.
pub(super) struct Inputs<T> {
    pub member: DeclarationId,
    pub parameters: Arc<[DeclarationId]>,
    pub arguments: Vec<T>,
    pub frame: Vec<nir::Sort>,
}

impl<T: fmt::Display> fmt::Display for Inputs<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(trait-defaults (member {}) (inputs", self.member)?;
        for (parameter, argument) in self.parameters.iter().zip(&self.arguments) {
            write!(f, " ({parameter} {argument})")?;
        }
        f.write_str(") (frame")?;
        for sort in &self.frame {
            write!(f, " {sort}")?;
        }
        f.write_str("))")
    }
}

impl<'world> Interpreter<'world> {
    pub(super) fn method_inputs<'m>(
        &mut self,
        method: comptime_compiler::methods::View<'m>,
    ) -> Result<(Inputs<&'m Term>, &'world nir::program::MethodContract), Error> {
        let (parameters, contract) = self
            .method_contracts
            .get(&(method.interface, method.member))
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(method.source)))?;
        if parameters.len() != 1 + method.interface_arguments.len() + method.method_arguments.len()
            || method.method_arguments.len() != contract.type_parameters.len()
        {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(method.source)));
        }
        let parameters = parameters.clone();
        self.integer_operation(Some(method.source), |budget| {
            budget.consume(parameters.len())
        })?;
        let arguments = std::iter::once(method.subject)
            .chain(method.interface_arguments)
            .chain(method.method_arguments)
            .collect();
        let signature = contract
            .signature
            .ordinary()
            .map_err(|_| self.failure(ErrorKind::InvalidCallPlan, Some(method.source)))?;
        let frame = self.ordinary_input_frame(signature, method.source)?;
        Ok((
            Inputs {
                member: method.member,
                parameters,
                arguments,
                frame,
            },
            contract,
        ))
    }
}
