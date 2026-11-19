//! Project a coherent implementation world onto the code a required root can
//! observe. Keep completed associated data for type theory and method links
//! for slot checking; only compatible initializer programs enter this source.
use super::*;

pub(super) struct TraitWorld {
    pub(super) traits: Vec<n::Trait>,
    pub(super) implementations: Vec<n::Implementation>,
    pub(super) method_links: Vec<n::MethodLink>,
}

impl Checker<'_, '_> {
    pub(super) fn required_trait_world(
        &mut self,
        retained: &BTreeSet<Dependency>,
        span: Span,
    ) -> Result<TraitWorld, Error> {
        let mut traits = Vec::with_capacity(self.checked_traits.len());
        let mut method_names = BTreeMap::new();
        for interface in self.checked_traits.iter() {
            let mut methods = Vec::new();
            for method in &interface.methods {
                if retained.contains(&Dependency::InterfaceMember(Member::Method(
                    method.declaration,
                ))) {
                    method_names
                        .entry(interface.declaration)
                        .or_insert_with(BTreeSet::new)
                        .insert(method.name.as_str());
                    methods.push(method.clone());
                }
            }
            traits.push(n::Trait {
                declaration: interface.declaration,
                name: interface.name.clone(),
                comptime_only: interface.comptime_only,
                self_parameter: interface.self_parameter,
                type_parameters: interface.type_parameters.clone(),
                parameter_types: interface.parameter_types.clone(),
                parents: interface.parents.clone(),
                requirements: interface.requirements.clone(),
                capabilities: interface.capabilities.clone(),
                associated: interface.associated.clone(),
                methods,
            });
        }

        let mut implementations = Vec::with_capacity(self.checked_implementations.len());
        let mut targets = BTreeSet::new();
        for row in &self.checked_implementations {
            let names = row.interface.and_then(|id| method_names.get(&id));
            let mut methods = Vec::new();
            for (name, body) in &row.methods {
                if names.is_some_and(|names| names.contains(name.as_str())) {
                    targets.insert(*body);
                    methods.push((name.clone(), *body));
                }
            }
            let mut associated_initializers = Vec::new();
            self.required_steps = self
                .required_steps
                .checked_sub(row.associated_initializers.len() as u64)
                .ok_or_else(|| {
                    error(
                        span,
                        "required associated program directory budget exhausted",
                    )
                })?;
            for (member, program) in &row.associated_initializers {
                if retained.contains(&Dependency::ImplementationMember {
                    implementation: row.declaration,
                    member: Member::Associated(*member),
                }) {
                    associated_initializers.push((*member, program.clone()));
                }
            }
            implementations.push(n::Implementation {
                declaration: row.declaration,
                interface: row.interface,
                phases: row.phases,
                interface_arguments: row.interface_arguments.clone(),
                type_parameters: row.type_parameters.clone(),
                parameter_types: row.parameter_types.clone(),
                requirements: row.requirements.clone(),
                capabilities: row.capabilities.clone(),
                subject: row.subject.clone(),
                associated: row.associated.clone(),
                associated_initializers,
                methods,
            });
        }
        for method in traits
            .iter_mut()
            .flat_map(|interface| &mut interface.methods)
        {
            if method.default.is_some_and(|id| {
                !targets.contains(&id) && !retained.contains(&Dependency::Definition(id))
            }) {
                method.default = None;
            }
        }
        let method_links = targets
            .into_iter()
            .filter(|id| !retained.contains(&Dependency::Definition(*id)))
            .map(|id| self.required_method_link(id, span))
            .collect::<Result<_, _>>()?;
        Ok(TraitWorld {
            traits,
            implementations,
            method_links,
        })
    }

    fn required_method_link(
        &mut self,
        id: DeclarationId,
        span: Span,
    ) -> Result<n::MethodLink, Error> {
        let (source, phase, is_quote, parameters) =
            if let Some(function) = self.generated_functions.declaration(id) {
                (
                    function.source,
                    function.phase,
                    false,
                    &function.type_parameters,
                )
            } else {
                let owner = self
                    .source_declarations
                    .owner(id)
                    .ok_or_else(|| error(span, "implementation method declaration is missing"))?;
                let item = self
                    .package
                    .item(owner)
                    .expect("indexed source declaration");
                let ItemKind::Fn(signature, _) = &item.kind else {
                    return Err(error(span, "implementation method is not a function"));
                };
                (
                    source(item.span),
                    function_bodies::phase(&signature.modifiers),
                    signature.modifiers.is_quote,
                    &self.type_parameters[&owner],
                )
            };
        self.required_steps = self
            .required_steps
            .checked_sub(1 + parameters.len() as u64)
            .ok_or_else(|| error(span, "implementation method link budget exhausted"))?;
        Ok(n::MethodLink {
            source,
            declaration: id,
            phase,
            is_quote,
            type_parameters: parameters.clone(),
        })
    }
}
