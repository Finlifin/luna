//! Resolve contextual values and executable type expressions before value dispatch.
use super::*;

impl Checker<'_, '_> {
    #[inline(never)]
    pub(super) fn special_value_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<Option<n::Expr>, Error> {
        if let H::ConstantObjectEntry(value, index) = expr.kind {
            return self
                .constant_object_entry(value, index, expected, expr.span)
                .map(Some);
        }
        if self.describing_open_spread && matches!(expr.kind, H::InlineControl(_)) {
            return self
                .describe_contextual_inline_input(expr, expected, partial)
                .map(Some);
        }
        if let Some(value) = self.projected_member_expression(expr, expected)? {
            return Ok(Some(value));
        }
        if matches!(expr.kind, H::TySelf) {
            let ty = self.self_type(expr.hir_id.owner, expr.span)?;
            let id = match ty {
                Term::Nominal { declaration, .. } | Term::Rigid { declaration, .. } => {
                    Some(declaration)
                }
                _ => None,
            };
            if let Some(binding) = id
                .and_then(|id| self.alias_value_bindings.get(&id))
                .copied()
            {
                let ty = p(P::Type);
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind: K::Local(binding),
                }));
            }
        }
        let (head, inputs) = match &expr.kind {
            H::NFApplication(head, args) => (*head, *args),
            H::ResolvedDef { .. } => (expr, &[][..]),
            _ => (expr, &[][..]),
        };
        if let H::ResolvedDef { def_id, .. } = head.kind {
            if let Some(owner) = self.package.definition_owner(def_id) {
                if self
                    .nominal_captures(owner, expr.span)?
                    .iter()
                    .any(|id| self.alias_value_bindings.contains_key(id))
                {
                    return self
                        .defaulted_type_expression(owner, inputs, expr.span, expected)
                        .map(Some);
                }
            }
        }
        if let H::CodeSplice(operand, substitutions) = &expr.kind {
            self.record_quote_hole(expr.span)?;
            return self
                .connect_code(expr, operand, substitutions, expected)
                .map(Some);
        }
        if matches!(expr.kind, H::CodeQuote(..)) {
            return Err(error(
                expr.span,
                "quoted code is a comptime value; bind it with const or supply it to a code splice",
            ));
        }
        if let H::ResolvedDef { def_id, .. } = expr.kind {
            if self.package.definition_owner(def_id).is_some_and(|owner| {
                self.is_code_generator(owner) || self.is_pattern_generator(owner)
            }) {
                return Err(error(
                    expr.span,
                    "code generator must be invoked in a comptime code position",
                ));
            }
        }
        if let H::Resume(value) = &expr.kind {
            let ty = self.effect_resume_result.clone().ok_or_else(|| {
                error(
                    expr.span,
                    "resume requires an enclosing direct effect branch",
                )
            })?;
            let value = if let Some(value) = value {
                self.expr(value, Some(&ty))?
            } else {
                self.require(&p(P::Void), &ty, expr.span)?;
                n::Expr {
                    source: source(expr.span),
                    ty: p(P::Void),
                    kind: K::Unit,
                }
            };
            return Ok(Some(n::Expr {
                source: source(expr.span),
                ty: p(P::Never),
                kind: K::EffectResume(Box::new(value)),
            }));
        }
        if let H::ErrorHandle {
            value,
            before,
            arms,
            after,
        } = &expr.kind
        {
            return self
                .error_handle(expr, value, *before, arms, *after, expected, partial)
                .map(Some);
        }
        if let H::EffectHandle(value, arms) = &expr.kind {
            return self.effect_handle(expr, value, arms, expected).map(Some);
        }
        if let H::EffectRequest(call) = &expr.kind {
            return self.effect_request(expr, call, expected).map(Some);
        }
        if let H::ResolvedClauseParam(_, id) = expr.kind {
            let declaration = DeclarationId {
                package: semantic_package(&self.package_identities, id.pkg),
                local: id.index,
            };
            if self.forbidden_default_inputs.contains(&declaration) {
                return Err(error(
                    expr.span,
                    "normal-form default references itself or a later parameter",
                ));
            }
            if let Some((binding, ty)) = self.normal_default_bindings.get(&declaration).cloned() {
                if let Some(Some(value)) = self.generator_data.get(&binding) {
                    constants::charge_copy(value, &mut self.required_steps, expr.span)?;
                    let mut value = value.clone();
                    if matches!(value.kind, K::Block { .. }) {
                        self.freshen_constant_graph(&mut value, expr.span)?;
                    }
                    if let Some(expected) = expected {
                        self.require_expression(&value.ty, expected, expr.span)?;
                    }
                    return Ok(Some(value));
                }
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind: K::Local(binding),
                }));
            }
            if let Some(binding) = self.alias_value_bindings.get(&declaration).copied() {
                let ty = p(P::Type);
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                for frame in &mut self.captures {
                    if frame.outer.contains(&binding)
                        && !frame
                            .values
                            .iter()
                            .any(|capture| capture.binding == binding)
                    {
                        frame.values.push(n::Capture {
                            binding,
                            ty: ty.clone(),
                            value: None,
                        });
                    }
                }
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind: K::Local(binding),
                }));
            }
            if let Some(domain) = self
                .trait_input_domain(declaration)
                .filter(|ty| *ty != p(P::Type))
                && let Some(value) = self.alias_substitution.get(&declaration).cloned()
            {
                let ty = self.substituted_trait_domain(&domain, expr.span)?;
                let binding = self.spread_binding(expr.span)?;
                let root = n::Expr {
                    source: source(expr.span),
                    ty: ty.clone(),
                    kind: K::Local(binding),
                };
                let value = self.copy_alias_type(&value, expr.span)?;
                let (value, ty) =
                    self.required_normal_input(expr, root, vec![(binding, ty, value)])?;
                let value = if let comptime_interpretor::Value::AbstractValue {
                    declaration,
                    ty: domain,
                } = value
                {
                    // A forwarded clause is still a generic value input.
                    // Preserve its declaration until instance substitution;
                    // freezing it as completed generator data loses staging.
                    self.require_expression(&domain, &ty, expr.span)?;
                    n::Expr {
                        source: source(expr.span),
                        ty,
                        kind: K::ComptimeValue(Term::Rigid {
                            declaration,
                            sort: Sort::ComptimeValue,
                        }),
                    }
                } else {
                    self.materialize_generator_data(value, ty, expr.span)?
                };
                if let Some(expected) = expected {
                    self.require_expression(&value.ty, expected, expr.span)?;
                }
                return Ok(Some(value));
            }
            if let Some((_, binding, ty)) = self
                .normal_parameters
                .values()
                .flatten()
                .find(|(d, _, ty)| *d == declaration && *ty != p(P::Type))
            {
                if let Some(expected) = expected {
                    self.require(ty, expected, expr.span)?;
                }
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty: ty.clone(),
                    kind: K::Local(*binding),
                }));
            }
            // Container value parameters are generic inputs, not function locals.
            // Keep their identity until the selected callable is instantiated.
            if self.signature_owner.is_some_and(|owner| {
                // Associated defaults share the trait's input telescope, but
                // must retain the existing symbolic associated-member lookup.
                let associated_parent = self
                    .package
                    .item(owner)
                    .filter(|item| matches!(item.kind, ItemKind::Associated(_)))
                    .and_then(|_| {
                        self.package.owners().find_map(|(parent, info)| {
                            match &info.node.expect_item().kind {
                                ItemKind::Trait(def) if def.items.contains(&owner) => Some(parent),
                                _ => None,
                            }
                        })
                    });
                std::iter::once(owner)
                    .chain(self.self_containers.get(&owner).copied())
                    .chain(associated_parent)
                    .any(|scope| {
                        self.type_parameters
                            .get(&scope)
                            .is_some_and(|parameters| parameters.contains(&declaration))
                            || self.nominal_capture_ids(scope).contains(&declaration)
                    })
            }) && let Some(domain) = self
                .trait_input_domain(declaration)
                .filter(|ty| *ty != p(P::Type))
            {
                let ty = self.substituted_trait_domain(&domain, expr.span)?;
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind: K::ComptimeValue(Term::Rigid {
                        declaration,
                        sort: Sort::ComptimeValue,
                    }),
                }));
            }
        }
        // Type observations form values without evaluating the observed term.
        // Only the explicit builtin BitVec head is an executable constructor;
        // ordinary normal-form heads remain opaque.
        if let H::NFApplication(head, args) = &expr.kind {
            if matches!(&head.kind, H::Builtin(name) if name.as_str() == "BitVec") {
                let [Arg::Positional(width)] = *args else {
                    return Err(error(
                        expr.span,
                        "builtin BitVec requires exactly one positional comptime width",
                    ));
                };
                let width = self.expr(width, Some(&p(P::Integer)))?;
                let ty = if is_never(&width.ty) {
                    p(P::Never)
                } else {
                    p(P::Type)
                };
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                return Ok(Some(n::Expr {
                    ty,
                    source: source(expr.span),
                    kind: K::BitVecType(Box::new(width)),
                }));
            }
        }
        if let H::NFApplication(head, arguments) = &expr.kind {
            if let Some(owner) = self.normal_form_constant_target(head)? {
                return self
                    .normal_type_expression(expr, owner, arguments, expected)
                    .map(Some);
            }
            if let H::ResolvedDef { def_id, .. } = head.kind {
                if let Some(owner) = self.package.definition_owner(def_id) {
                    if (self.has_optional_type_clauses(owner)
                        || self
                            .trait_domains(owner)?
                            .iter()
                            .any(|domain| *domain != p(P::Type))
                        || arguments.iter().any(|arg| matches!(arg, Arg::Expand(_))))
                        && matches!(
                            self.package.item(owner).map(|item| &item.kind),
                            Some(
                                ItemKind::Struct(_)
                                    | ItemKind::Enum(_)
                                    | ItemKind::Newtype(..)
                                    | ItemKind::TypeAlias(..)
                            )
                        )
                    {
                        return self
                            .defaulted_type_expression(owner, arguments, expr.span, expected)
                            .map(Some);
                    }
                    if matches!(
                        self.package.item(owner).map(|item| &item.kind),
                        Some(ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..))
                    ) {
                        let parameters = self.type_parameters[&owner].clone();
                        if parameters.is_empty() || parameters.len() != arguments.len() {
                            return Err(error(
                                expr.span,
                                "generic type type argument count mismatch",
                            ));
                        }
                        let mut values = Vec::new();
                        let mut terms = Vec::new();
                        let mut deferred = BTreeSet::new();
                        for (argument, parameter) in arguments.iter().zip(&parameters) {
                            let Arg::Positional(value) = argument else {
                                return Err(error(expr.span, "type arguments must be positional"));
                            };
                            let value = self.expr(value, Some(&p(P::Type)))?;
                            if let K::TypeValue(ty) = &value.kind {
                                terms.push(ty.clone());
                            } else {
                                deferred.insert(*parameter);
                                terms.push(Term::Rigid {
                                    declaration: *parameter,
                                    sort: Sort::Type,
                                });
                            }
                            values.push(value);
                        }
                        let dynamic = !deferred.is_empty();
                        self.check_formation_application(
                            owner,
                            FormationInputs {
                                terms: terms.clone(),
                                deferred,
                                environment: BTreeMap::new(),
                            },
                            expr.span,
                        )?;
                        let ty = if values.iter().any(|value| is_never(&value.ty)) {
                            p(P::Never)
                        } else {
                            p(P::Type)
                        };
                        if let Some(expected) = expected {
                            self.require_expression(&ty, expected, expr.span)?;
                        }
                        let captures = self.nominal_capture_arguments(owner, expr.span)?;
                        terms.extend(captures.iter().cloned());
                        values.extend(captures.into_iter().map(|capture| n::Expr {
                            source: source(expr.span),
                            ty: p(P::Type),
                            kind: K::TypeValue(capture),
                        }));
                        let kind = if dynamic {
                            self.ensure_nominal_expression(owner, expr.span)?;
                            K::NominalType {
                                preparations: vec![],
                                bindings: None,
                                order: None,
                                declaration: declaration(
                                    self.package,
                                    &self.package_identities,
                                    owner,
                                ),
                                arguments: values
                                    .into_iter()
                                    .map(n::NormalFormArgument::Value)
                                    .collect(),
                            }
                        } else {
                            K::TypeValue(Term::Nominal {
                                declaration: declaration(
                                    self.package,
                                    &self.package_identities,
                                    owner,
                                ),
                                arguments: terms,
                            })
                        };
                        if self.in_required_comptime {
                            if let K::TypeValue(ty) = &kind {
                                self.ensure_invocation_type(ty, expr.span)?;
                            }
                        }
                        return Ok(Some(n::Expr {
                            source: source(expr.span),
                            ty,
                            kind,
                        }));
                    }
                    if let ItemKind::TypeAlias(value, ..) = &self.package.item(owner).unwrap().kind
                    {
                        let parameters = self.type_parameters[&owner].clone();
                        if parameters.is_empty() || parameters.len() != arguments.len() {
                            return Err(error(
                                expr.span,
                                "generic type alias argument count mismatch",
                            ));
                        }
                        let mut values = Vec::new();
                        for argument in *arguments {
                            let Arg::Positional(value) = argument else {
                                return Err(error(
                                    expr.span,
                                    "type alias arguments must be positional",
                                ));
                            };
                            values.push(self.expr(value, Some(&p(P::Type)))?);
                        }
                        let result =
                            self.alias_value_expression(owner, value, values, expr.span)?;
                        if let Some(expected) = expected {
                            self.require(&result.ty, expected, expr.span)?;
                        }
                        return Ok(Some(result));
                    }
                    if matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_normal_form_head)
                    {
                        return self
                            .normal_type_expression(expr, owner, arguments, expected)
                            .map(Some);
                    }
                }
            }
        }
        if function_types::has_named_inputs(expr) {
            let value = self.type_expr(expr)?;
            if let Some(expected) = expected {
                self.require_expression(&p(P::Type), expected, expr.span)?;
            }
            return Ok(Some(n::Expr {
                source: source(expr.span),
                ty: p(P::Type),
                kind: K::TypeValue(value),
            }));
        }
        match &expr.kind {
            H::TraitBound(..) if self.executable_refinement_owner == Some(expr.hir_id.owner) => {
                let value = self.refinement_trait_value(expr)?;
                let ty = p(P::Bool);
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind: K::Literal(n::Literal::Bool(value)),
                }));
            }
            H::TyClosure(context, _) if matches!(context.kind, H::ResolvedClauseParam(..)) => {
                // A clause context is a symbolic declaration input, not a
                // runtime environment construction. Keep the complete type
                // value so specialization substitutes its context and body
                // together before runtime materialization.
                let value = self.type_expr(expr)?;
                let ty = p(P::Type);
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind: K::TypeValue(value),
                }));
            }
            H::RefinedType { .. } | H::RefinementSubject => {
                let value = self.type_expr(expr)?;
                let ty = p(P::Type);
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind: K::TypeValue(value),
                }));
            }
            H::TyOptional(inner) => {
                let inner = self.expr(inner, Some(&p(P::Type)))?;
                let ty = inner.ty.clone();
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                let kind = match inner.kind {
                    K::TypeValue(value) => K::TypeValue(Term::Optional(Box::new(value))),
                    _ => K::OptionalType(Box::new(inner)),
                };
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind,
                }));
            }
            H::TyPtr(inner) => {
                let inner = self.expr(inner, Some(&p(P::Type)))?;
                let ty = inner.ty.clone();
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                let kind = match inner.kind {
                    K::TypeValue(value) => K::TypeValue(Term::Pointer(Box::new(value))),
                    _ => K::PointerType(Box::new(inner)),
                };
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind,
                }));
            }
            H::TyClosure(context, body) => {
                let H::TyClosureEnvironment(bindings) = &context.kind else {
                    return Err(error(
                        context.span,
                        "open closure contexts require environment-parameter elaboration not yet implemented",
                    ));
                };
                let mut names = std::collections::HashSet::new();
                let mut environment = Vec::new();
                for binding in *bindings {
                    if !names.insert(binding.name.name) {
                        return Err(error(
                            binding.name.span,
                            "duplicate closure environment binding",
                        ));
                    }
                    if binding.value.is_some() {
                        return Err(error(
                            binding.name.span,
                            "closure environment sources require dependency/provenance checking not yet implemented",
                        ));
                    }
                    environment.push(self.expr(binding.ty, Some(&p(P::Type)))?);
                }
                let signature = self.expr(body, Some(&p(P::Type)))?;
                let ty = if environment
                    .iter()
                    .chain(std::iter::once(&signature))
                    .any(|field| is_never(&field.ty))
                {
                    p(P::Never)
                } else {
                    p(P::Type)
                };
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                if environment.is_empty() {
                    return Ok(Some(signature));
                }
                if let K::TypeValue(body) = &signature.kind {
                    if matches!(body, Term::Closure { .. }) {
                        return Err(error(
                            expr.span,
                            "a type cannot have repeated nonempty closure qualification",
                        ));
                    }
                    if !matches!(
                        body,
                        Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)
                    ) {
                        return Err(error(
                            expr.span,
                            "non-callable closure-qualified bodies are not yet implemented",
                        ));
                    }
                }
                let kind = if environment
                    .iter()
                    .all(|field| matches!(field.kind, K::TypeValue(_)))
                    && matches!(signature.kind, K::TypeValue(_))
                {
                    let environment = environment
                        .into_iter()
                        .map(|field| {
                            let K::TypeValue(ty) = field.kind else {
                                unreachable!()
                            };
                            ty
                        })
                        .collect();
                    let K::TypeValue(body) = signature.kind else {
                        unreachable!()
                    };
                    K::TypeValue(Term::Closure {
                        environment,
                        signature: Box::new(body),
                    })
                } else {
                    K::ClosureType {
                        environment,
                        signature: Box::new(signature),
                    }
                };
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind,
                }));
            }
            H::TyModifiedFn(..) => {
                if let Some(expected) = expected {
                    self.require_expression(&p(P::Type), expected, expr.span)?;
                }
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty: p(P::Type),
                    kind: K::TypeValue(self.type_expr(expr)?),
                }));
            }
            H::TyFn(parameters) | H::TyComptimeFn(parameters) => {
                let comptime = matches!(expr.kind, H::TyComptimeFn(_));
                let mut fields = Vec::new();
                let mut comptime_parameters = Vec::new();
                for parameter in *parameters {
                    if parameter.flags & !hir::common::TPARAM_COMPTIME != 0 {
                        return Err(error(
                            expr.span,
                            "function type parameter protocols are not implemented",
                        ));
                    }
                    let TyParamKind::Positional(value) = parameter.kind else {
                        return Err(error(
                            expr.span,
                            "function type parameter form is not implemented",
                        ));
                    };
                    fields.push(self.expr(value, Some(&p(P::Type)))?);
                    comptime_parameters.push(parameter.is_comptime());
                }
                let ty = if fields.iter().any(|field| is_never(&field.ty)) {
                    p(P::Never)
                } else {
                    p(P::Type)
                };
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                let kind = if fields
                    .iter()
                    .all(|field| matches!(field.kind, K::TypeValue(_)))
                {
                    let parameters = fields
                        .into_iter()
                        .zip(&comptime_parameters)
                        .map(|(field, staged)| {
                            let K::TypeValue(ty) = field.kind else {
                                unreachable!()
                            };
                            Parameter {
                                sort: if *staged {
                                    if ty == p(P::Type) {
                                        Sort::Type
                                    } else {
                                        Sort::ComptimeValue
                                    }
                                } else {
                                    Sort::Runtime
                                },
                                ty,
                            }
                        })
                        .collect();
                    K::TypeValue(
                        Term::Function {
                            parameters,
                            result: Box::new(p(P::Void)),
                        }
                        .with_comptime_phase(comptime),
                    )
                } else {
                    K::FunctionType {
                        comptime,
                        comptime_parameters,
                        parameters: fields,
                    }
                };
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind,
                }));
            }
            H::TyFnArrow(input, output) => {
                let signature = self.expr(input, Some(&p(P::Type)))?;
                let result = self.expr(output, Some(&p(P::Type)))?;
                let ty = if is_never(&signature.ty) || is_never(&result.ty) {
                    p(P::Never)
                } else {
                    p(P::Type)
                };
                if let Some(expected) = expected {
                    self.require_expression(&ty, expected, expr.span)?;
                }
                let kind = if let (K::TypeValue(input), K::TypeValue(output)) =
                    (&signature.kind, &result.kind)
                {
                    K::TypeValue(
                        input
                            .clone()
                            .with_callable_result(output.clone())
                            .ok_or_else(|| {
                                error(expr.span, "arrow input must be a function type")
                            })?,
                    )
                } else {
                    K::FunctionResultType {
                        signature: Box::new(signature),
                        result: Box::new(result),
                    }
                };
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind,
                }));
            }
            _ => {}
        }
        if let (H::List(items), Some(expected)) = (&expr.kind, expected) {
            if let Some(element) = self.list_element_type(expected).cloned() {
                self.required_steps = self
                    .required_steps
                    .checked_sub(items.len() as u64)
                    .ok_or_else(|| error(expr.span, "type-list construction budget exhausted"))?;
                let fields = items
                    .iter()
                    .map(|item| self.expr(item, Some(&element)))
                    .collect::<Result<Vec<_>, _>>()?;
                let ty = if fields.iter().any(|field| is_never(&field.ty)) {
                    p(P::Never)
                } else {
                    expected.clone()
                };
                return Ok(Some(n::Expr {
                    source: source(expr.span),
                    ty,
                    kind: K::TypeList(fields),
                }));
            }
        }
        if expected.is_none() && matches!(expr.kind, H::List(_)) {
            return Err(context_error(
                expr.span,
                "list literal requires an element type context",
            ));
        }
        if expected == Some(&p(P::Type)) && matches!(expr.kind, H::Tuple(_) | H::Unit) {
            return Err(error(
                expr.span,
                "a parenthesized tuple is a value, not a type; use Tuple<...> to construct a tuple type",
            ));
        }
        let type_value = self.is_type_head(expr)
            || matches!(
                expr.kind,
                H::TypeOf(_)
                    | H::Builtin(_)
                    | H::TyType
                    | H::TyVoid
                    | H::TyNoReturn
                    | H::TyPtr(_)
                    | H::TyOptional(_)
                    | H::TyClosure(..)
                    | H::TyFn(_)
                    | H::TyComptimeFn(_)
                    | H::TyFnArrow(..)
            );
        if type_value {
            let ty = p(P::Type);
            if let Some(expected) = expected {
                self.require_expression(&ty, expected, expr.span)?;
            }
            let value = self.type_expr(expr)?;
            if self.in_required_comptime {
                // Early invocations can transport and reflect a nominal Type
                // before the ordinary declaration pass has built its descriptor.
                self.ensure_invocation_type(&value, expr.span)?;
            }
            return Ok(Some(n::Expr {
                ty,
                source: source(expr.span),
                kind: K::TypeValue(value),
            }));
        }
        Ok(None)
    }
}
