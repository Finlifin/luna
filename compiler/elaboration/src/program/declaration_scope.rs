//! Lexical entry for declaration queries. Signatures, contracts, defaults and
//! bodies bind the requested declaration's inputs; open checking can import
//! supplied input evidence. The caller retains its expression delivery,
//! control targets and PTC assumptions until the query returns.
//! Declaration publication and shared work budgets belong to the checker.

use super::*;

impl Checker<'_, '_> {
    pub(super) fn in_declaration_scope<R>(
        &mut self,
        owner: OwnerId,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        self.with_lexical_scope(owner, None, input_facts::State::default(), check)
    }

    /// Enter a declaration without substitutions from the requesting expression.
    /// Defaults bind their formal prefix; projected initializers install the
    /// selected container inputs before checking their source.
    pub(super) fn in_formal_declaration<R>(
        &mut self,
        owner: OwnerId,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        self.in_declaration_scope(owner, |this| {
            let types = std::mem::take(&mut this.alias_substitution);
            let values = std::mem::take(&mut this.alias_value_bindings);
            let bindings = std::mem::take(&mut this.normal_default_bindings);
            let forbidden = std::mem::take(&mut this.forbidden_default_inputs);
            let result = check(this);
            this.forbidden_default_inputs = forbidden;
            this.alias_substitution = types;
            this.alias_value_bindings = values;
            this.normal_default_bindings = bindings;
            result
        })
    }

    pub(super) fn in_open_function_scope<T>(
        &mut self,
        owner: OwnerId,
        instance: Option<std::sync::Arc<generated_functions::InstanceKey>>,
        check: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let inputs = self.input_facts.for_open_body();
        self.with_lexical_scope(owner, instance, inputs, |this| {
            this.context = if this.open_instance.is_some() {
                this.definition_context_in_scope(owner, this.call_input_frames())
            } else {
                this.definition_context(owner)
            };
            check(this)
        })
    }

    fn with_lexical_scope<T>(
        &mut self,
        owner: OwnerId,
        instance: Option<std::sync::Arc<generated_functions::InstanceKey>>,
        inputs: input_facts::State,
        check: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let saved_alias_inputs = std::mem::take(&mut self.abstract_alias_types);
        let saved_refinements = std::mem::take(&mut self.definition_refinements);
        let saved_behavior_inputs = std::mem::take(&mut self.ordinary_behavior_inputs);
        let saved_behavior = std::mem::replace(&mut self.checking_open_behavior, false);
        let saved_call = std::mem::replace(&mut self.describing_open_call, false);
        let saved_spread = std::mem::replace(&mut self.describing_open_spread, false);
        let body_declaration = instance
            .as_ref()
            .and_then(|key| self.generated_functions.instance(key))
            .or_else(|| Some(declaration(self.package, &self.package_identities, owner)));
        let saved_body_declaration =
            std::mem::replace(&mut self.active_body_declaration, body_declaration);
        let saved_list_infos = std::mem::take(&mut self.list_pattern_infos);
        let saved_range_infos = std::mem::take(&mut self.range_pattern_infos);
        let saved_bitvec_infos = std::mem::take(&mut self.bitvec_pattern_infos);
        let saved_owner = self.signature_owner.replace(owner);
        let saved_instance = std::mem::replace(&mut self.open_instance, instance);
        let saved_instance_capabilities = self.open_instance_capabilities.take();
        let saved_inline = self.inline_selection.take();
        let saved_match_input = self.inline_match_input.take();
        let saved_operand_scope = self.operand_scope.take();
        let saved_unselected = std::mem::replace(&mut self.in_unselected_inline, false);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_local_type_views = std::mem::take(&mut self.local_type_views);
        let saved_code = std::mem::take(&mut self.in_code_body);
        let saved_unsafe = std::mem::replace(&mut self.unsafe_depth, 0);
        let saved_holes = std::mem::take(&mut self.quote_holes);
        let pattern_quote = self.pattern_quote_pending.take();
        let checking_generator = std::mem::take(&mut self.checking_code_generator);
        let generator_data = std::mem::take(&mut self.generator_data);
        let generator_cells = std::mem::take(&mut self.generator_cells);
        let dependencies = std::mem::take(&mut self.abstract_code_dependencies);
        let saved_code_values = std::mem::take(&mut self.code_values);
        let saved_patterns = std::mem::take(&mut self.pattern_codes);
        let saved_staged_locals = std::mem::take(&mut self.open_staged_locals);
        let saved_staged_locals_members = std::mem::take(&mut self.open_member_locals);
        let pending_initializer = self.pending_initializer.take();
        let saved_captures = std::mem::take(&mut self.captures);
        let saved_control = std::mem::take(&mut self.control);
        let saved_loop_transfers = std::mem::take(&mut self.open_loop_transfers);
        let saved_immutable = std::mem::take(&mut self.immutable);
        let saved_context = std::mem::take(&mut self.context);
        let saved_contract = std::mem::replace(&mut self.checking_contract, false);
        let saved_numeric = std::mem::replace(&mut self.numeric_defaults, true);
        let saved_staged = std::mem::replace(&mut self.in_staged_input, false);
        let saved_required = std::mem::replace(&mut self.in_required_comptime, false);
        let saved_inputs = std::mem::replace(&mut self.input_facts, inputs);
        let saved_default_scope = self.default_input_scope.take();
        let saved_default_owner = self.default_input_owner.take();
        let saved_observation = std::mem::take(&mut self.observing_type);
        let saved_effect_delivery = std::mem::take(&mut self.preserve_effect_value);
        let saved_callee_delivery = self.comptime_callee_result.take();
        let saved_result_context = self.structural_result_context.take();
        let saved_effects = self.effect_environment.take();
        let saved_resume = self.effect_resume_result.take();
        let saved_local_runtime_origins = std::mem::take(&mut self.local_runtime_origins);
        let saved_local_runtime_deliveries = std::mem::take(&mut self.local_runtime_deliveries);
        let saved_local_write_versions = std::mem::take(&mut self.local_write_versions);
        let saved_active_env_parameters = std::mem::take(&mut self.active_env_parameters);
        let result = check(self);
        self.active_env_parameters = saved_active_env_parameters;
        self.local_write_versions = saved_local_write_versions;
        self.local_runtime_deliveries = saved_local_runtime_deliveries;
        self.local_runtime_origins = saved_local_runtime_origins;
        self.abstract_alias_types = saved_alias_inputs;
        self.definition_refinements = saved_refinements;
        self.ordinary_behavior_inputs = saved_behavior_inputs;
        self.checking_open_behavior = saved_behavior;
        self.describing_open_call = saved_call;
        self.describing_open_spread = saved_spread;
        self.active_body_declaration = saved_body_declaration;
        self.list_pattern_infos = saved_list_infos;
        self.range_pattern_infos = saved_range_infos;
        self.bitvec_pattern_infos = saved_bitvec_infos;
        self.signature_owner = saved_owner;
        self.open_instance = saved_instance;
        self.open_instance_capabilities = saved_instance_capabilities;
        self.inline_selection = saved_inline;
        self.inline_match_input = saved_match_input;
        self.operand_scope = saved_operand_scope;
        self.in_unselected_inline = saved_unselected;
        self.locals = saved_locals;
        self.local_type_views = saved_local_type_views;
        self.pattern_codes = saved_patterns;
        self.code_values = saved_code_values;
        self.abstract_code_dependencies = dependencies;
        self.generator_cells = generator_cells;
        self.generator_data = generator_data;
        self.checking_code_generator = checking_generator;
        self.pattern_quote_pending = pattern_quote;
        self.quote_holes = saved_holes;
        self.unsafe_depth = saved_unsafe;
        self.in_code_body = saved_code;
        self.open_staged_locals = saved_staged_locals;
        self.open_member_locals = saved_staged_locals_members;
        self.pending_initializer = pending_initializer;
        self.captures = saved_captures;
        self.control = saved_control;
        self.open_loop_transfers = saved_loop_transfers;
        self.immutable = saved_immutable;
        self.context = saved_context;
        self.checking_contract = saved_contract;
        self.numeric_defaults = saved_numeric;
        self.in_staged_input = saved_staged;
        self.in_required_comptime = saved_required;
        self.input_facts = saved_inputs;
        self.default_input_scope = saved_default_scope;
        self.default_input_owner = saved_default_owner;
        self.observing_type = saved_observation;
        self.preserve_effect_value = saved_effect_delivery;
        self.comptime_callee_result = saved_callee_delivery;
        self.structural_result_context = saved_result_context;
        self.effect_environment = saved_effects;
        self.effect_resume_result = saved_resume;
        result
    }
}
