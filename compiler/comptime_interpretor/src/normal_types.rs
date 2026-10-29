//! Invocation-local state for sequential normal-form construction.
use super::*;

pub(super) struct Inputs {
    pub head: DeclarationId,
    pub arguments: Vec<Term>,
    parameters: Vec<nir::DeclarationParameter>,
}
impl Inputs {
    pub fn new(head: DeclarationId) -> Self {
        Self {
            head,
            arguments: vec![],
            parameters: vec![],
        }
    }
}
impl fmt::Display for Inputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(normal-inputs (head {}) (arguments", self.head)?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, "))")
    }
}
impl Interpreter<'_> {
    pub(super) fn prepare_normal_type(
        &mut self,
        head: DeclarationId,
        ty: &Term,
        count: usize,
        diverges: bool,
        source: SourceRange,
    ) -> Result<(), Error> {
        let metadata = self
            .normal_forms
            .get(&head)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let expected = if diverges {
            Primitive::Never
        } else {
            Primitive::Type
        };
        if *ty != Term::Primitive(expected) || count != metadata.parameters.len() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        self.integer_operation(Some(source), |budget| budget.consume(count))
    }
    pub(super) fn prepare_compiled_normal_input(
        &mut self,
        inputs: &Inputs,
        index: usize,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        let metadata = *self
            .normal_forms
            .get(&inputs.head)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        if index != inputs.arguments.len() || index >= metadata.parameters.len() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let expected = self.instantiate_contextual_domain(
            &metadata.parameters[index].ty,
            &inputs.parameters,
            &inputs.arguments,
            source,
        )?;
        self.prepare_normal_value_type(ty, &expected, source)
    }
    pub(super) fn prepare_normal_value_type(
        &mut self,
        ty: &Term,
        expected: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if ty != expected && *ty != Term::Primitive(Primitive::Never) {
            self.check_type_contract(
                &nir::program::TypePredicate::Equal(ty.clone(), expected.clone()),
                source,
            )?;
        }
        Ok(())
    }
    pub(super) fn bind_compiled_normal_input(
        &mut self,
        inputs: &mut Inputs,
        value: Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        let parameter = self
            .normal_forms
            .get(&inputs.head)
            .and_then(|head| head.parameters.get(inputs.arguments.len()))
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let parameter = nir::DeclarationParameter {
            declaration: parameter.declaration,
            sort: if parameter.kind == nir::normal_forms::Kind::Type {
                nir::Sort::Type
            } else {
                nir::Sort::ComptimeValue
            },
        };
        let term = self.residual_input_term(value, source)?;
        inputs.parameters.push(parameter);
        inputs.arguments.push(term);
        Ok(())
    }
}
