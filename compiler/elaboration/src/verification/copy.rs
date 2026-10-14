//! Type-level Copy classification for subsequent resource passes. The protocol
//! identity is supplied by language-item binding, never guessed from a name.
//! A selected Copy impl still needs the independent representation/resource
//! legality check; this query does not certify duplication of cleanup rights.
use crate::{
    capabilities::World,
    implementation::{Selection, TypeAssessment},
    ptc::Budget,
};
use nir::{DeclarationId, Primitive, Term, program::InterfaceInstance};
use std::fmt;

/// Copy capability and cleanup evidence acquired in one immutable world.
/// This describes supported representations; value-specific reachability and
/// access checks remain with the ownership pass performing the actual transfer.
#[derive(Clone, Debug)]
pub struct Evidence {
    components: Vec<(Term, Classification)>,
    cleanup: super::drop::AcquiredCleanup,
}
impl Evidence {
    pub fn ty(&self) -> &Term {
        self.cleanup.ty()
    }
}
impl fmt::Display for Evidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(copy-evidence (components")?;
        for (ty, classification) in &self.components {
            write!(f, " (component (type {ty}) {classification})")?;
        }
        write!(f, ") {})", self.cleanup)
    }
}

/// Require an explicit capability for every non-intrinsic component. Checking
/// all enum variants prevents the active tag from hiding an affine alternative.
/// Nothing is cached across implementation worlds or resource assumptions.
pub fn acquire(
    package: &nir::program::Package,
    world: &World<'_>,
    copy_protocol: DeclarationId,
    drop_protocol: DeclarationId,
    ty: &Term,
    budget: Budget,
    fuel: &mut usize,
) -> Result<Evidence, String> {
    let declaration = world
        .traits
        .iter()
        .find(|t| t.declaration == copy_protocol)
        .ok_or("Copy protocol identity is not a declared trait")?;
    if package
        .traits
        .iter()
        .find(|t| t.declaration == copy_protocol)
        != Some(declaration)
    {
        return Err("Copy protocol package and query world disagree".into());
    }
    let cleanup = super::drop::acquire_cleanup(package, world, drop_protocol, ty, budget, fuel)?;
    let mut components = Vec::new();
    cleanup.check_trivial_components(fuel, &mut |ty, fuel| {
        super::memory::bound_type(ty, fuel)?;
        let classification = classify(
            world,
            copy_protocol,
            ty,
            Budget {
                steps: budget.steps.min(*fuel),
                ..budget
            },
        )?;
        match &classification {
            Classification::Intrinsic | Classification::Declared { .. } => {}
            Classification::Assumed(_) if matches!(ty, Term::Function { .. }) => {}
            Classification::Assumed(_) => {
                return Err("Copy representation requires concrete capability evidence".into());
            }
            Classification::Affine => {
                return Err(format!("Copy component has no Copy capability: {ty}"));
            }
            Classification::Pending(reason) => {
                return Err(format!("Copy component is unresolved: {reason}"));
            }
        }
        if !matches!(classification, Classification::Intrinsic)
            && nir::normal_forms::tuple_fields(Some(package), ty).is_none()
            && !matches!(
                ty,
                Term::Nominal { .. } | Term::Tuple(_) | Term::Record(_) | Term::Function { .. } | Term::Closure { .. }
            )
        {
            return Err("Copy representation is not established for this value category".into());
        }
        components.push((ty.clone(), classification));
        Ok(())
    })?;
    Ok(Evidence {
        components,
        cleanup,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Classification {
    Intrinsic,
    Assumed(nir::program::TraitRequirement),
    Declared {
        implementation: DeclarationId,
        arguments: Vec<Term>,
    },
    Affine,
    Pending(String),
}
impl fmt::Display for Classification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Assumed(requirement) => {
                write!(f, "(copy-classification (assumed {requirement}))")
            }
            Self::Intrinsic => f.write_str("(copy-classification intrinsic)"),
            Self::Affine => f.write_str("(copy-classification affine)"),
            Self::Declared {
                implementation,
                arguments,
            } => {
                write!(
                    f,
                    "(copy-classification (implementation {implementation}) (arguments"
                )?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str("))")
            }
            Self::Pending(reason) => write!(
                f,
                "(copy-classification (pending {}))",
                Term::String(reason.clone().into())
            ),
        }
    }
}
/// Query only the named protocol in this immutable world. No decision is cached
/// across changes to implementations or assumptions. Nominal field shape never
/// creates an implicit Copy declaration.
pub fn classify(
    world: &World<'_>,
    protocol: DeclarationId,
    ty: &Term,
    budget: Budget,
) -> Result<Classification, String> {
    let contract = world
        .traits
        .iter()
        .find(|t| t.declaration == protocol)
        .ok_or("Copy protocol identity is not a declared trait")?;
    if !contract.type_parameters.is_empty() {
        return Err("Copy protocol must have no explicit type parameters".into());
    }
    if budget.steps == 0 {
        return Ok(Classification::Pending(
            "Copy query budget exhausted".into(),
        ));
    }
    // Only the intrinsic classes whose copy behavior is specified independently
    // of user impls. Managed text, closures and arbitrary Type values are not
    // inferred Copy merely because the reference evaluator can clone them.
    if matches!(
        ty,
        Term::Pointer(_)
            | Term::Primitive(
                Primitive::Bool
                    | Primitive::Char
                    | Primitive::Symbol
                    | Primitive::Integer
                    | Primitive::Float
                    | Primitive::BitVec(_)
                    | Primitive::I8
                    | Primitive::I16
                    | Primitive::I32
                    | Primitive::I64
                    | Primitive::I128
                    | Primitive::U8
                    | Primitive::U16
                    | Primitive::U32
                    | Primitive::U64
                    | Primitive::U128
                    | Primitive::Isize
                    | Primitive::Usize
                    | Primitive::F32
                    | Primitive::F64
            )
    ) {
        return Ok(Classification::Intrinsic);
    }
    let requirement = nir::program::TraitRequirement {
        interface: protocol,
        interface_arguments: vec![],
        subject: ty.clone(),
    };
    if let Some(given) = world.given(&requirement, budget) {
        return Ok(Classification::Assumed(given));
    }
    let resolution = world.resolve_instance(
        &InterfaceInstance {
            declaration: protocol,
            arguments: vec![],
        },
        ty,
        budget,
    )?;
    Ok(match resolution.types.selection {
        Selection::Selected(implementation) => {
            let Some(TypeAssessment::Applicable(evidence)) =
                resolution.types.assessments.get(&implementation)
            else {
                return Err("Copy selection lacks instantiated type evidence".into());
            };
            Classification::Declared {
                implementation,
                arguments: evidence.type_arguments.clone(),
            }
        }
        Selection::NoApplicableCandidate if world.assumptions.is_empty() => Classification::Affine,
        Selection::NoApplicableCandidate => {
            Classification::Pending("Copy assumptions were not established".into())
        }
        unresolved => Classification::Pending(format!("{unresolved}")),
    })
}
