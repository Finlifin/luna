//! Evaluate construction packets before entering checked host publication.
use super::*;
use comptime_compiler::definitions::{Build, Kind};

impl Interpreter<'_> {
    pub(super) fn check_definition_build(
        &self,
        output: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        if !matches!(output, Term::Primitive(Primitive::Type | Primitive::Never)) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(())
    }

    pub(super) fn complete_definition_build(
        &mut self,
        build: &Build<impl std::borrow::Borrow<Term>>,
        value: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if *build.output.borrow() != Term::Primitive(Primitive::Type) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        match build.kind {
            Kind::Struct => self.build_struct_value(value, source),
            Kind::Enum => self.build_enum_value(value, source),
        }
    }

    pub(super) fn eval_definition_build(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (kind, request) = match &expr.kind {
            K::BuildStruct(request) => (Kind::Struct, request),
            K::BuildEnum(request) => (Kind::Enum, request),
            _ => unreachable!(),
        };
        self.check_definition_build(&expr.ty, expr.source)?;
        let value = match self.eval(request, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        self.complete_definition_build(
            &Build {
                kind,
                output: &expr.ty,
            },
            value,
            expr.source,
        )
        .map(Flow::Value)
    }
}
