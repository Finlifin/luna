//! Evaluate construction packets before entering checked host publication.
use super::*;
use comptime_compiler::definitions::{Build, Kind};

impl<'a> Interpreter<'a> {
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
        let request = self.decode_definition_build(build.kind, value, source)?;
        let resolver = self.definition_resolver(&request);
        let prepared = host_budget::clamped(&mut self.budget.steps, |steps| {
            resolver.prepare(&request, steps)
        })?;
        self.admit_definition_build(&request, prepared)
    }

    pub(super) fn decode_definition_build(
        &mut self,
        kind: Kind,
        value: Value,
        source: SourceRange,
    ) -> Result<DefinitionBuildRequest, Error> {
        match kind {
            Kind::Struct => self
                .decode_struct_build(value, source)
                .map(DefinitionBuildRequest::Struct),
            Kind::Enum => self
                .decode_enum_build(value, source)
                .map(DefinitionBuildRequest::Enum),
            Kind::Newtype => self
                .decode_newtype_build(value, source)
                .map(DefinitionBuildRequest::Newtype),
        }
    }

    pub(super) fn definition_resolver(&self, request: &DefinitionBuildRequest) -> Resolver<'a> {
        match request {
            DefinitionBuildRequest::Struct(_) => Resolver::Struct(self.struct_builder),
            DefinitionBuildRequest::Enum(_) => Resolver::Enum(self.enum_builder),
            DefinitionBuildRequest::Newtype(_) => Resolver::Newtype(self.newtype_builder),
        }
    }

    pub(super) fn admit_definition_build(
        &mut self,
        request: &DefinitionBuildRequest,
        prepared: PreparedDefinitionBuild<'a>,
    ) -> Result<Value, Error> {
        match (request, prepared) {
            (
                DefinitionBuildRequest::Struct(request),
                PreparedDefinitionBuild::Struct(prepared),
            ) => self.admit_struct_build(request, prepared),
            (DefinitionBuildRequest::Enum(request), PreparedDefinitionBuild::Enum(prepared)) => {
                self.admit_enum_build(request, prepared)
            }
            (
                DefinitionBuildRequest::Newtype(request),
                PreparedDefinitionBuild::Newtype(prepared),
            ) => self.admit_newtype_build(request, prepared),
            _ => Err(self.failure(ErrorKind::Type, Some(request.source()))),
        }
    }

    pub(super) fn eval_definition_build(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (kind, request) = match &expr.kind {
            K::BuildStruct(request) => (Kind::Struct, request),
            K::BuildNewtype(request) => (Kind::Newtype, request),
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

/// Validated source descriptors; no generated declaration is installed yet.
#[derive(Debug)]
pub enum DefinitionBuildRequest {
    Struct(StructBuildRequest),
    Enum(EnumBuildRequest),
    Newtype(NewtypeBuildRequest),
}
impl DefinitionBuildRequest {
    pub fn source(&self) -> SourceRange {
        match self {
            Self::Struct(r) => r.source,
            Self::Enum(r) => r.source,
            Self::Newtype(r) => r.source,
        }
    }
}
impl fmt::Display for DefinitionBuildRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Struct(r) => r.fmt(f),
            Self::Enum(r) => r.fmt(f),
            Self::Newtype(r) => r.fmt(f),
        }
    }
}
/// The receiving VM must admit the exact descriptor before committing it.
pub enum PreparedDefinitionBuild<'a> {
    Struct(PreparedStructBuild<'a>),
    Enum(PreparedEnumBuild<'a>),
    Newtype(PreparedNewtypeBuild<'a>),
}

pub(super) enum Resolver<'a> {
    Struct(
        Option<&'a dyn Fn(&StructBuildRequest, &mut u64) -> Result<PreparedStructBuild<'a>, Error>>,
    ),
    Enum(Option<&'a dyn Fn(&EnumBuildRequest, &mut u64) -> Result<PreparedEnumBuild<'a>, Error>>),
    Newtype(
        Option<
            &'a dyn Fn(&NewtypeBuildRequest, &mut u64) -> Result<PreparedNewtypeBuild<'a>, Error>,
        >,
    ),
}
impl<'a> Resolver<'a> {
    pub(super) fn prepare(
        &self,
        request: &DefinitionBuildRequest,
        fuel: &mut u64,
    ) -> Result<PreparedDefinitionBuild<'a>, Error> {
        match (self, request) {
            (Self::Struct(Some(prepare)), DefinitionBuildRequest::Struct(r)) => {
                prepare(r, fuel).map(PreparedDefinitionBuild::Struct)
            }
            (Self::Enum(Some(prepare)), DefinitionBuildRequest::Enum(r)) => {
                prepare(r, fuel).map(PreparedDefinitionBuild::Enum)
            }
            (Self::Newtype(Some(prepare)), DefinitionBuildRequest::Newtype(r)) => {
                prepare(r, fuel).map(PreparedDefinitionBuild::Newtype)
            }
            _ => Err(Error {
                kind: ErrorKind::Type,
                source: Some(request.source()),
            }),
        }
    }
}
