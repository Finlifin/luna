//! Charged request assembly for the shared concrete publisher.
use super::CheckedDefinitionPublisher;
use comptime_interpretor::{
    Budget, EnumBuildRequest, Error, ErrorKind, PreparedEnumBuild, PreparedStructBuild,
    StructBuildRequest,
};
use nir::{DeclarationId, program as n};
use std::sync::Arc;

impl CheckedDefinitionPublisher<'_> {
    pub fn prepare_struct_request<'s>(
        &'s self,
        identity: DeclarationId,
        request: &StructBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        self.set_diagnostic(None);
        let mut budget = Budget {
            steps: std::mem::take(steps),
            depth: 128,
        };
        let result = (|| {
            charge_request(&mut budget, |fuel| request.charge_assembly(fuel))?;
            let definitions: Arc<[n::Struct]> = vec![n::Struct {
                declaration: identity,
                name: request.name.clone(),
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                fields: request
                    .fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.clone()))
                    .collect(),
            }]
            .into();
            let prepared =
                self.prepare_structs(Arc::clone(&definitions), request.source, &mut budget)?;
            Ok(PreparedStructBuild {
                definitions,
                result: identity,
                commit: Box::new(move || prepared.commit()),
            })
        })();
        *steps = budget.steps;
        self.request_result(result, request.source)
    }
    pub fn prepare_enum_request<'s>(
        &'s self,
        identity: DeclarationId,
        request: &EnumBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedEnumBuild<'s>, Error> {
        self.set_diagnostic(None);
        let mut budget = Budget {
            steps: std::mem::take(steps),
            depth: 128,
        };
        let result = (|| {
            charge_request(&mut budget, |fuel| request.charge_assembly(fuel))?;
            let definitions: Arc<[n::Enum]> = vec![n::Enum {
                declaration: identity,
                name: request.name.clone(),
                is_intrinsic: false,
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                groups: request.groups.clone(),
                variants: request.variants.clone(),
                encoding: request.encoding.clone(),
                symbol_encoding: request.symbol_encoding.clone(),
            }]
            .into();
            let prepared =
                self.prepare_enums(Arc::clone(&definitions), request.source, &mut budget)?;
            Ok(PreparedEnumBuild {
                definitions,
                result: identity,
                commit: Box::new(move || prepared.commit()),
            })
        })();
        *steps = budget.steps;
        self.request_result(result, request.source)
    }
    fn request_result<T>(
        &self,
        result: Result<T, Error>,
        source: n::SourceRange,
    ) -> Result<T, Error> {
        result.map_err(|mut error| {
            error.source = Some(source);
            if self.last_diagnostic().is_none() {
                self.set_diagnostic(Some(format!("checked definition request failed: {error}")));
            }
            error
        })
    }
}
fn charge_request(
    budget: &mut Budget,
    charge: impl FnOnce(&mut usize) -> Result<(), ()>,
) -> Result<(), Error> {
    let before = usize::try_from(budget.steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let result = charge(&mut fuel);
    budget.steps -= (before - fuel) as u64;
    result.map_err(|_| Error {
        kind: ErrorKind::StepsExhausted,
        source: None,
    })
}
