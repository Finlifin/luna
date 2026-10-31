use iir::{
    InstanceKey,
    cleanup::{ContextKey, FunctionSchedule, Schedule, ScopeLookupError},
};
use nir::{DeclarationId, Primitive, Term, program::SourceRange};

fn scope(instance: InstanceKey, context: Vec<ContextKey>) -> FunctionSchedule {
    FunctionSchedule {
        instance,
        context,
        sites: vec![],
        discards: vec![],
        transfers: vec![],
        copies: vec![],
        pattern_transfers: vec![],
        pattern_orders: vec![],
    }
}
fn key(domain: Primitive) -> InstanceKey {
    InstanceKey {
        declaration: DeclarationId {
            package: 1,
            local: 2,
        },
        arguments: (vec![Term::Primitive(domain)]).into(),
    }
}
fn handler(arm: usize) -> ContextKey {
    ContextKey::EffectHandler {
        installation: SourceRange { start: 10, end: 20 },
        handler: 0,
        arm,
    }
}

#[test]
fn resource_scopes_distinguish_instance_installer_and_arm() {
    let first = key(Primitive::U8);
    let second = key(Primitive::U16);
    let parent = ContextKey::Default {
        construction: SourceRange { start: 1, end: 9 },
        parameter: 0,
    };
    let nested = vec![parent.clone(), handler(0)];
    let schedule = Schedule {
        functions: vec![
            scope(first.clone(), vec![handler(0)]),
            scope(first.clone(), vec![handler(1)]),
            scope(second.clone(), vec![handler(0)]),
            scope(first.clone(), nested.clone()),
        ],
        ..Default::default()
    };
    assert!(std::ptr::eq(
        schedule.exact_function(&first, &[handler(0)]).unwrap(),
        &schedule.functions[0]
    ));
    assert!(std::ptr::eq(
        schedule.exact_function(&first, &[handler(1)]).unwrap(),
        &schedule.functions[1]
    ));
    assert!(std::ptr::eq(
        schedule.exact_function(&second, &[handler(0)]).unwrap(),
        &schedule.functions[2]
    ));
    assert!(std::ptr::eq(
        schedule.exact_function(&first, &nested).unwrap(),
        &schedule.functions[3]
    ));
    assert_eq!(
        schedule.exact_function(&first, &[handler(2)]).unwrap_err(),
        ScopeLookupError::Missing
    );
}

#[test]
fn duplicate_scope_is_rejected_even_when_contents_match() {
    let instance = key(Primitive::U8);
    let context = vec![handler(0)];
    let mut schedule = Schedule {
        functions: vec![scope(instance.clone(), context.clone())],
        ..Default::default()
    };
    schedule.functions.push(schedule.functions[0].clone());
    assert_eq!(
        schedule.exact_function(&instance, &context).unwrap_err(),
        ScopeLookupError::Ambiguous
    );
    schedule.functions.pop();
    assert!(schedule.exact_function(&instance, &context).is_ok());
    assert_eq!(
        ScopeLookupError::Ambiguous.to_string(),
        "(cleanup-scope-error ambiguous)"
    );
}

#[test]
fn alias_does_not_supply_an_absent_activation_scope() {
    let instance = key(Primitive::U8);
    let context = vec![handler(0)];
    let schedule = Schedule {
        functions: vec![scope(instance.clone(), vec![])],
        aliases: vec![iir::cleanup::ContextAlias {
            instance: instance.clone(),
            context: context.clone(),
            target: vec![],
        }],
        ..Default::default()
    };
    assert_eq!(
        schedule.exact_function(&instance, &context).unwrap_err(),
        ScopeLookupError::Missing
    );
}
