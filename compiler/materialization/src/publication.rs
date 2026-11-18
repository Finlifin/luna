//! Meter retained graph metadata before exposing the completed IIR package.
use super::*;

fn spend(fuel: &mut usize, amount: usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(amount)
        .ok_or("IIR publication metadata budget exhausted")?;
    Ok(())
}

fn charge_key(arguments: &[Term], fuel: &mut usize) -> Result<(), String> {
    spend(fuel, arguments.len().saturating_add(1))?;
    for argument in arguments {
        nir::type_computations::charge_term(argument, fuel).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Index executable and stored targets once rather than scanning every template
/// table for each dispatch method. The structural arguments remain part of the
/// identity; a matching declaration number alone never retains a method.
pub(super) fn retain_demanded_methods(
    functions: &[iir::FunctionInstance],
    templates: &[iir::template_storage::TemplateStorage],
    dispatch: &mut [n::DispatchInstance],
    fuel: &mut usize,
) -> Result<(), String> {
    let mut demanded = HashSet::new();
    for function in functions {
        charge_key(&function.key.arguments, fuel)?;
        demanded.insert((function.key.declaration, function.key.arguments.as_ref()));
    }
    for table in templates {
        spend(fuel, 1)?;
        for target in table.targets() {
            charge_key(&target.arguments, fuel)?;
            demanded.insert((target.declaration, target.arguments.as_ref()));
        }
    }
    // Finish reservations before mutating any method list. All records still
    // belong to the private graph; final IIR validation remains mandatory.
    for interface in dispatch.iter() {
        spend(fuel, 1)?;
        for method in &interface.methods {
            charge_key(&method.arguments, fuel)?;
        }
    }
    for interface in dispatch {
        interface
            .methods
            .retain(|method| demanded.contains(&(method.body, method.arguments.as_slice())));
    }
    Ok(())
}

/// Normal-form heads stay opaque. Copy metadata without interpreting bodies or
/// giving lists, tuples or any other concrete head a special identity rule.
pub(super) fn copy_normal_forms(
    heads: &[nir::normal_forms::Head],
    fuel: &mut usize,
) -> Result<Vec<nir::normal_forms::Head>, String> {
    spend(fuel, heads.len())?;
    for head in heads {
        spend(
            fuel,
            head.parameters
                .len()
                .saturating_add(head.requirements.len()),
        )?;
        for parameter in &head.parameters {
            nir::copy_budget::charge_type_substitution(&parameter.ty, &[], &[], fuel)
                .map_err(|e| format!("IIR normal-form parameter copy limit: {e:?}"))?;
        }
        for requirement in &head.requirements {
            nir::copy_budget::charge_predicate_substitution(requirement, &[], &[], fuel)
                .map_err(|e| format!("IIR normal-form requirement copy limit: {e:?}"))?;
        }
    }
    Ok(heads.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::{Primitive, Sort};

    fn id(local: u32) -> Id {
        Id { package: 7, local }
    }

    #[test]
    fn normal_form_publication_charges_payloads_without_interpreting_them() {
        let heads = vec![nir::normal_forms::Head {
            declaration: id(1),
            role: Default::default(),
            collector: None,
            parameters: vec![nir::normal_forms::Parameter {
                declaration: id(2),
                binding: n::LocalId { owner: 1, index: 0 },
                kind: nir::normal_forms::Kind::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            requirements: vec![n::TypePredicate::Bool(false); 1000],
        }];
        let mut fuel = 10_000;
        assert_eq!(copy_normal_forms(&heads, &mut fuel).unwrap(), heads);
        let cost = 10_000 - fuel;
        assert!(cost > 1000);
        assert!(copy_normal_forms(&heads, &mut (cost - 1)).is_err());
        let mut exact = cost;
        assert_eq!(copy_normal_forms(&heads, &mut exact).unwrap(), heads);
        assert_eq!(exact, 0);
        let mut wider = heads.clone();
        wider[0].requirements.extend(heads[0].requirements.clone());
        assert!(copy_normal_forms(&wider, &mut cost.clone()).is_err());
    }

    #[test]
    fn retained_template_targets_use_full_identity_and_fail_before_pruning() {
        let signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::ComptimeValue,
                ty: Term::Primitive(Primitive::Bool),
            }],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let package = n::Package {
            runtime_function_identities: vec![n::RuntimeFunctionIdentity {
                requirements: vec![],
                capabilities: vec![],
                declaration: id(1),
                type_parameters: vec![id(2)],
                signature: signature.clone(),
            }],
            function_parameters: vec![nir::function_parameters::Declaration {
                declaration: id(1),
                parameters: vec![id(2)],
                domains: vec![Term::Primitive(Primitive::Bool)],
            }],
            ..Default::default()
        };
        let table = iir::template_storage::TemplateStorage::from_package(
            signature,
            vec![iir::InstanceKey {
                declaration: id(1),
                arguments: vec![Term::Bool(true)].into(),
            }],
            &package,
            &mut 10_000,
        )
        .unwrap();
        let method = |body, flag| n::MethodInstance {
            member: id(10),
            body,
            arguments: vec![Term::Bool(flag)],
            method_arguments: vec![],
        };
        let original = vec![n::DispatchInstance {
            interface: id(3),
            interface_arguments: vec![],
            subject: Term::Primitive(Primitive::Bool),
            implementation: id(4),
            arguments: vec![],
            methods: vec![
                method(id(1), false),
                method(id(1), true),
                method(id(5), true),
            ],
            associated: vec![],
            associated_initializers: vec![],
        }];
        let tables = [table];
        let mut result = original.clone();
        let mut fuel = 1000;
        retain_demanded_methods(&[], &tables, &mut result, &mut fuel).unwrap();
        assert_eq!(result[0].methods, vec![method(id(1), true)]);
        let cost = 1000 - fuel;
        for allowance in 0..cost {
            let mut result = original.clone();
            assert!(
                retain_demanded_methods(&[], &tables, &mut result, &mut allowance.clone()).is_err()
            );
            assert_eq!(result, original);
        }
        let mut exact = cost;
        let mut recovered = original;
        retain_demanded_methods(&[], &tables, &mut recovered, &mut exact).unwrap();
        assert_eq!(exact, 0);
        assert_eq!(recovered, result);
    }
}
