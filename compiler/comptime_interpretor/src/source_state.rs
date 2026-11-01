//! Validated source declarations separated from fresh invocation state.
use super::*;

pub(super) struct Prepared<'a> {
    pub(super) effect_declarations: declaration_index::Effects<'a>,
    pub(super) method_parameters: Arc<HashMap<(DeclarationId, DeclarationId), Vec<LocalId>>>,
    pub(super) declaration_parameter_sorts: Arc<BTreeMap<DeclarationId, nir::Sort>>,
    pub(super) declaration_parameter_domains: Arc<BTreeMap<DeclarationId, Term>>,
    pub(super) method_contracts: method_catalog::Contracts<'a>,
    pub(super) method_origins: Arc<HashMap<DeclarationId, Option<DeclarationId>>>,
    pub(super) implemented_interfaces: Arc<HashMap<(InterfaceInstance, Term), DeclarationId>>,
    pub(super) assumed_interfaces: HashSet<(InterfaceInstance, Term)>,
    pub(super) method_bodies:
        Arc<HashMap<(InterfaceInstance, Term, DeclarationId), (DeclarationId, Vec<Term>)>>,
    pub(super) associated_values: associated_catalog::Catalog<'a>,
    pub(super) associated_initializers: associated_catalog::Catalog<'a>,
    pub(super) functions: declaration_index::Functions<'a>,
    pub(super) runtime_function_identities: declaration_index::RuntimeIdentities<'a>,
    pub(super) structs: layout_catalog::Catalog<generated_structs::StructDefinition<'a>>,
    pub(super) enums: layout_catalog::Catalog<generated_enums::EnumDefinition<'a>>,
}

impl<'a> Interpreter<'a> {
    pub(super) fn from_source_indexes(
        package: &'a Package,
        mut budget: Budget,
        index: Arc<declaration_index::SourceIndexes>,
    ) -> Result<Self, Error> {
        budget.steps = budget.steps.checked_sub(1).ok_or(Error {
            kind: ErrorKind::StepsExhausted,
            source: None,
        })?;
        let prepared = Prepared {
            effect_declarations: declaration_index::Effects::new(
                &package.effects,
                Some(index.effects.clone()),
                |item| item.declaration,
            )?,
            method_parameters: index.method_parameters.clone(),
            declaration_parameter_sorts: index.parameter_sorts.clone(),
            declaration_parameter_domains: index.parameter_domains.clone(),
            method_contracts: method_catalog::Contracts {
                traits: &package.traits,
                entries: index.method_contracts.clone(),
            },
            method_origins: index.method_origins.clone(),
            implemented_interfaces: index.implementations.clone(),
            assumed_interfaces: HashSet::new(),
            method_bodies: index.methods.clone(),
            associated_values: associated_catalog::Catalog::indexed(
                package,
                index.associated.clone(),
            ),
            associated_initializers: associated_catalog::Catalog::indexed(
                package,
                index.initializers.clone(),
            ),
            functions: declaration_index::Functions::new(
                &package.functions,
                Some(index.functions.clone()),
                |item| item.declaration,
            )?,
            runtime_function_identities: declaration_index::RuntimeIdentities::new(
                &package.runtime_function_identities,
                Some(index.runtime_identities.clone()),
                |item| item.declaration,
            )?,
            structs: layout_catalog::Catalog::from_base(index.structs.clone()),
            enums: layout_catalog::Catalog::from_base(index.enums.clone()),
        };
        Self::from_source_state(package, budget, Some(index), prepared)
    }

    pub(super) fn from_source_state(
        package: &'a Package,
        budget: Budget,
        positions: Option<Arc<declaration_index::SourceIndexes>>,
        prepared: Prepared<'a>,
    ) -> Result<Self, Error> {
        let Prepared {
            effect_declarations,
            method_parameters,
            declaration_parameter_sorts,
            declaration_parameter_domains,
            method_contracts,
            method_origins,
            implemented_interfaces,
            assumed_interfaces,
            method_bodies,
            associated_values,
            associated_initializers,
            functions,
            runtime_function_identities,
            structs,
            enums,
        } = prepared;
        Ok(Self {
            bytecode_invocations: 0,
            bytecode_default_evaluations: 0,
            bytecode_staged_inputs: 0,
            bytecode_contextual_inputs: 0,
            bytecode_input_preparations: 0,
            bytecode_compilations: 0,
            bytecode_compilation_steps: 0,
            bytecode_cache: Default::default(),
            borrowed_code_cache: Default::default(),
            input_body_cache: Default::default(),
            default_code_cache: Default::default(),
            contextual_code_cache: Default::default(),
            prepared_instance_cache: Default::default(),
            declaration_world: Some(package),
            effect_declarations,
            effect_state: effects::State::default(),
            effect_values: None,
            graph: None,
            target: package.target,
            cleanup_schedule: None,
            cleanup_scopes: HashMap::new(),
            cleanup_sites: HashMap::new(),
            cleanup_effect_context: HashMap::new(),
            cleanup_frame: None,
            cleanup_failed: false,
            prepared_frames: vec![],
            residual_temporaries: vec![],
            pattern_binding_orders: BTreeMap::new(),
            cleanup_context: vec![],
            comptime_only: false,
            bytecode_enabled: true,
            abstract_type_parameters: BTreeSet::new(),
            abstract_value_parameters: BTreeMap::new(),
            struct_builder: None,
            enum_builder: None,
            type_solver: None,
            pointer_copy: None,
            method_parameters,
            declaration_parameter_sorts,
            declaration_parameter_domains,
            method_contracts,
            method_origins,
            implemented_interfaces,
            assumed_interfaces,
            method_bodies,
            associated_values,
            associated_initializers,
            active_associated_initializers: HashSet::new(),
            instances: Default::default(),
            functions,
            runtime_function_identities,
            graph_indexes: None,
            normal_forms: declaration_index::NormalForms::new(
                &package.normal_forms,
                positions.as_ref().map(|index| index.normal_forms.clone()),
                |head| head.declaration,
            )?,
            structs,
            alias_contracts: declaration_index::Aliases::new(
                &package.alias_contracts,
                positions.as_ref().map(|index| index.aliases.clone()),
                |contract| contract.declaration,
            )?,
            newtypes: declaration_index::Newtypes::new(
                &package.newtypes,
                positions.as_ref().map(|index| index.newtypes.clone()),
                |definition| definition.declaration,
            )?,
            enums,
            budget: Budget {
                steps: budget.steps,
                // Bound native recursion independently of a caller-supplied
                // budget: call frames include contracts and cleanup contexts.
                // This guard must fire on the supported 2 MiB host stack.
                depth: budget.depth.min(64),
            },
            depth: 0,
            invocation_depth: 0,
            invocation_depth_limit: budget.depth,
            events: Vec::new(),
        })
    }
}
