//! Private required invocation during concrete staged-input preparation.
use super::*;
use comptime_interpretor::{ContractOutcome, Interpreter, Value};

// Reuse checked code and metadata, never an interpreter, environment or result.
// The scratch root and dispatch table are cleared between independent invocations.
pub(super) struct Snapshot {
    package: n::Package,
    entry_index: usize,
}

impl Snapshot {
    fn clear(&mut self) {
        self.package.dispatch_instances.clear();
        let entry = &mut self.package.functions[self.entry_index];
        entry.signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(nir::Primitive::Void)),
        };
        entry.body = n::Expr {
            kind: K::Unit,
            ty: Term::Primitive(nir::Primitive::Void),
            source: n::SourceRange { start: 0, end: 0 },
        };
    }
}

impl Builder<'_> {
    pub(super) fn invoke_staged_input(&mut self, expression: &n::Expr) -> Result<n::Expr, String> {
        self.tick(0)?;
        let mut snapshot = match self.invocation_snapshot.take() {
            Some(snapshot) => snapshot,
            None => self.new_invocation_snapshot()?,
        };
        let result = self.invoke_with_snapshot(expression, &mut snapshot);
        snapshot.clear();
        self.invocation_snapshot = Some(snapshot);
        result
    }

    fn new_invocation_snapshot(&mut self) -> Result<Snapshot, String> {
        // The source package is already checked. Preserve its implementation
        // world in this private snapshot; publish only the frozen result.
        for function in &self.package.functions {
            self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or("staged snapshot budget exhausted")?;
            iir::visit::expressions(function, &mut self.fuel, |_, _| Ok(()))?;
            crate::associated::charge_projection_copy(&function.signature, &mut self.fuel)
                .map_err(|_| "staged snapshot type budget exhausted")?;
        }
        let mut snapshot = self.package.clone();
        let mut entry = Id {
            package: u64::MAX,
            local: u32::MAX,
        };
        while snapshot.functions.iter().any(|f| f.declaration == entry)
            || snapshot
                .runtime_function_identities
                .iter()
                .any(|f| f.declaration == entry)
        {
            entry.local = entry
                .local
                .checked_sub(1)
                .ok_or("no staged invocation identity available")?;
        }
        let entry_index = snapshot.functions.len();
        snapshot.functions.push(n::Function {
            declaration: entry,
            name: "<staged-input>".into(),
            phase: n::FunctionPhase::Comptime,
            inline: false, pure: false,
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(nir::Primitive::Void)),
            },
            parameters: vec![],
            type_parameters: vec![],
            body: n::Expr {
                kind: K::Unit,
                ty: Term::Primitive(nir::Primitive::Void),
                source: n::SourceRange { start: 0, end: 0 },
            },
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
        });
        snapshot.dispatch_instances.clear();
        Ok(Snapshot {
            package: snapshot,
            entry_index,
        })
    }

    fn invoke_with_snapshot(
        &mut self,
        expression: &n::Expr,
        scratch: &mut Snapshot,
    ) -> Result<n::Expr, String> {
        iir::visit::expression(expression, &mut self.fuel, |_, _| Ok(()))?;
        crate::associated::charge_projection_copy(&expression.ty, &mut self.fuel)
            .map_err(|_| "staged result type budget exhausted")?;
        let snapshot = &mut scratch.package;
        let root = &mut snapshot.functions[scratch.entry_index];
        root.signature = Term::Function {
            parameters: vec![],
            result: Box::new(expression.ty.clone()),
        };
        root.body = expression.clone();
        let entry = root.declaration;
        let roots = [iir::InstanceKey {
            declaration: entry,
            arguments: vec![],
        }];
        let prepared = build(
            snapshot,
            Budget {
                steps: self.fuel,
                ..self.budget
            },
            Roots::Dispatch(&roots),
            BTreeSet::new(),
            &[],
            Some((&self.context, &self.assumptions)),
        )?;
        snapshot.dispatch_instances = prepared.0;
        self.fuel = prepared.4;
        let solver = |predicate: &n::TypePredicate| {
            let world = crate::capabilities::World {
                index: &self.index,
                implementations: &snapshot.implementations,
                traits: &snapshot.traits,
                assumptions: &self.assumptions,
                context: &self.context,
            };
            match world.check_predicate(predicate, self.budget) {
                Ok(crate::ptc::Outcome::Entailed(_)) => ContractOutcome::Satisfied,
                Ok(crate::ptc::Outcome::NotEntailed(_)) => ContractOutcome::Rejected,
                _ => ContractOutcome::Unknown,
            }
        };
        let mut interpreter = Interpreter::for_comptime_with_assumptions(
            snapshot,
            &self.assumptions,
            comptime_interpretor::Budget {
                steps: self.fuel as u64,
                depth: 128,
            },
        )
        .map_err(|error| format!("staged comptime preparation: {error}"))?
        .with_type_solver(&solver);
        let value = interpreter.invoke(entry, vec![]);
        self.fuel = usize::try_from(interpreter.remaining_steps()).unwrap_or(usize::MAX);
        let value = value.map_err(|error| format!("staged comptime invocation: {error}"))?;
        self.freeze_staged_value(value, &expression.ty, expression.source, 0)
    }

    fn freeze_staged_value(
        &mut self,
        value: Value,
        ty: &Term,
        source: n::SourceRange,
        depth: usize,
    ) -> Result<n::Expr, String> {
        self.tick(depth)?;
        let kind = match value {
            Value::Unit => K::Unit,
            Value::Bool(v) => K::Literal(n::Literal::Bool(v)),
            Value::Char(v) => K::Literal(n::Literal::Char(v)),
            Value::Float32(bits) if *ty == Term::Primitive(nir::Primitive::F32) =>
                K::Literal(n::Literal::Float32(bits)),
            Value::Float64(bits) if *ty == Term::Primitive(nir::Primitive::F64) =>
                K::Literal(n::Literal::Float64(bits)),
            Value::Symbol(v) => {
                self.fuel = self
                    .fuel
                    .checked_sub(v.len())
                    .ok_or("staged string budget exhausted")?;
                K::Literal(n::Literal::Symbol(v.to_string()))
            }
            Value::String(v) => {
                self.fuel = self
                    .fuel
                    .checked_sub(v.len())
                    .ok_or("staged string budget exhausted")?;
                K::Literal(n::Literal::String(v.to_string()))
            }
            Value::SignedInteger(v) => K::Literal(n::Literal::SignedInteger(v)),
            Value::UnsignedInteger(v) => K::Literal(n::Literal::UnsignedInteger(v)),
            Value::Type(v) => K::TypeValue(v),
            Value::TypeList(types) => {
                if !nir::normal_forms::type_list_input_type(ty)
                    || !self.package.normal_forms.iter().any(|head|head.parameters.iter().any(|parameter|
                        parameter.kind==nir::normal_forms::Kind::TypeList && parameter.ty==*ty))
                {
                    return Err("staged type-list result requires trusted meta.List<Type>".into());
                }
                for element in &types {
                    iir::completed_type(element,&BTreeSet::new(),&mut self.fuel)?;
                    if element.sort()!=nir::Sort::Type {return Err("staged type-list element is not Type".into());}
                }
                K::TypeListValue(types)
            }
            Value::BitVec(v) => {
                self.fuel = self
                    .fuel
                    .checked_sub(v.words().len())
                    .ok_or("staged bit-vector budget exhausted")?;
                K::Literal(n::Literal::BitVec {
                    width: v.width(),
                    words: v.words().to_vec(),
                })
            }
            Value::Integer(v) => {
                let mut budget = comptime_runtime::Budget {
                    steps: self.fuel,
                    max_limbs: 4096,
                };
                let spelling = v.to_hex_literal(&mut budget);
                self.fuel = budget.steps;
                K::Literal(n::Literal::Integer(spelling.map_err(|e| e.to_string())?))
            }
            Value::AnonymousRecord(values) => {
                let Term::Record(types) = ty else { return Err("staged record result type mismatch".into()); };
                if values.len() != types.len() { return Err("staged record result arity mismatch".into()); }
                let mut fields = Vec::new();
                for ((name, value), (expected_name, ty)) in values.into_iter().zip(types) {
                    self.fuel = self.fuel.checked_sub(name.len()).ok_or("staged record budget exhausted")?;
                    if name != *expected_name { return Err("staged record result label mismatch".into()); }
                    fields.push(self.freeze_staged_value(value, ty, source, depth + 1)?);
                }
                K::AnonymousRecord { fields }
            }
            Value::Tuple(values) => {
                let Some(types) = nir::normal_forms::tuple_fields(Some(self.package), ty) else {
                    return Err("staged tuple result type mismatch".into());
                };
                if types.len() != values.len() {
                    return Err("staged tuple result arity mismatch".into());
                }
                K::Tuple(
                    values
                        .into_iter()
                        .zip(types)
                        .map(|(v, t)| self.freeze_staged_value(v, t, source, depth + 1))
                        .collect::<Result<_, _>>()?,
                )
            }
            _ => {
                return Err(
                    "staged invocation result cannot yet be frozen as an instance input".into(),
                );
            }
        };
        Ok(n::Expr {
            kind,
            ty: ty.clone(),
            source,
        })
    }
}
