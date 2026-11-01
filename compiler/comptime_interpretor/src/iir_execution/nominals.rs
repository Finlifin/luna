//! A live concrete view owns generated batches and borrows immutable artifacts.
//! It is an ownership adapter, not a cached formation or layout verdict.
use super::*;

pub(crate) enum ConcreteNominal<'a> {
    Struct(generated_structs::StructDefinition<'a>),
    Enum(generated_enums::EnumDefinition<'a>),
    Newtype(generated_newtypes::NewtypeDefinition<'a>),
}
impl<'a> ConcreteNominal<'a> {
    pub(super) fn borrowed(definition: &'a iir::NominalDefinition) -> Self {
        match definition {
            iir::NominalDefinition::Struct(definition) => {
                Self::Struct(generated_structs::StructDefinition::Borrowed(definition))
            }
            iir::NominalDefinition::Enum(definition) => {
                Self::Enum(generated_enums::EnumDefinition::Borrowed(definition))
            }
            iir::NominalDefinition::Newtype(definition) => {
                Self::Newtype(generated_newtypes::NewtypeDefinition::Borrowed(definition))
            }
        }
    }
}

/// Reserve every retained declaration identity, including body-free contracts.
/// Generated definitions remain session-local and are never inserted here.
pub(super) fn declaration_identities(
    graph: &iir::Package,
    fuel: &mut usize,
) -> Result<HashSet<DeclarationId>, ()> {
    let mut result = HashSet::new();
    let mut insert = |id| {
        *fuel = fuel.checked_sub(1).ok_or(())?;
        result.insert(id);
        Ok::<_, ()>(())
    };
    for nominal in &graph.nominals {
        if let Term::Nominal { declaration, .. } = &nominal.ty {
            insert(*declaration)?;
        }
    }
    for function in &graph.functions {
        insert(function.key.declaration)?;
    }
    for callable in &graph.callable_identities {
        insert(callable.key.declaration)?;
    }
    for head in &graph.normal_forms {
        insert(head.declaration)?;
        for parameter in &head.parameters {
            insert(parameter.declaration)?;
        }
    }
    for interface in &graph.interfaces {
        insert(interface.interface.declaration)?;
        for method in &interface.methods {
            insert(method.declaration)?;
            for parameter in &method.type_parameters {
                insert(*parameter)?;
            }
            if let Some(default) = method.default {
                insert(default)?;
            }
        }
    }
    for dispatch in &graph.dispatch {
        insert(dispatch.interface)?;
        insert(dispatch.implementation)?;
        for method in &dispatch.methods {
            insert(method.member)?;
            insert(method.body)?;
        }
        for (member, _) in &dispatch.associated {
            insert(*member)?;
        }
        for (member, _) in &dispatch.associated_initializers {
            insert(*member)?;
        }
    }
    for effect in &graph.effect_contracts {
        insert(effect.operation.declaration)?;
    }
    for template in &graph.templates {
        for target in template.targets() {
            insert(target.declaration)?;
        }
    }
    Ok(result)
}
