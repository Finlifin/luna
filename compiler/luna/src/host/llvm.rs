mod bits;
mod callables;
mod cleanup;
mod closures;
mod defaults;
mod effects;
mod entry;
mod enums;
mod errors;
mod frames;
mod named_calls;
mod optional;
use bits::bit_width;
mod decimal;
mod patterns;
mod pointers;
mod records;
mod strings;
mod text_queries;
use nir::program::{Expr, ExprKind as K, Function, IntegerOp as Op, Literal, LocalId};
use nir::{Primitive as P, Sort, Term};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Ty {
    Unit,
    Optional(std::sync::Arc<Ty>),
    ErrorResult(std::sync::Arc<Vec<(Option<Term>, Ty)>>),
    Bool,
    String,
    Symbol,
    Int(u32, bool),
    Float16,
    BFloat16,
    Float32,
    Float64,
    // A target representation only: IIR validation retains full function types.
    // Capturing environments and foreign ABIs do not use this representation.
    FunctionPointer,
    Closure(std::sync::Arc<Vec<Ty>>),
    // Table-local identity tags are not callable machine addresses.
    TemplateIdentity(std::sync::Arc<Term>),
    Enum(std::sync::Arc<enums::EnumLayout>),
    // Data addresses have no recursive physical pointee. Each access obtains
    // its layout from the checked logical IIR type, preserving nominal identity.
    Pointer,
    Record(std::sync::Arc<Vec<Ty>>),
}
impl Ty {
    fn llvm(&self) -> String {
        match self {
            Self::Optional(payload) => format!(
                "{{ i1, {} }}",
                if **payload == Ty::Unit {
                    "{}".into()
                } else {
                    payload.llvm()
                }
            ),
            Self::ErrorResult(fields) => format!(
                "{{ i32, {} }}",
                fields
                    .iter()
                    .map(|(_, ty)| if *ty == Ty::Unit {
                        "{}".into()
                    } else {
                        ty.llvm()
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Enum(layout) if layout.symbols.is_some() => Self::String.llvm(),
            Self::Enum(layout) if layout.aggregate => {
                let payloads = layout
                    .fields
                    .iter()
                    .map(|fields| {
                        Ty::Record(std::sync::Arc::new(
                            fields.iter().map(|(_, ty)| ty.clone()).collect(),
                        ))
                        .llvm()
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{ i32, {payloads} }}")
            }
            Self::Enum(layout) => format!("i{}", layout.width),
            Self::Closure(fields) => format!(
                "{{ ptr, {}, [{} x i1] }}",
                Self::Record(fields.clone()).llvm(),
                closures::state_count(fields)
            ),
            Self::Record(fields) => format!(
                "{{ {} }}",
                fields
                    .iter()
                    .map(|ty| if *ty == Ty::Unit {
                        "{}".into()
                    } else {
                        ty.llvm()
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Pointer | Self::FunctionPointer => "ptr".into(),
            Self::TemplateIdentity(_) => "i64".into(),
            Self::Float16 => "half".into(),
            Self::BFloat16 => "bfloat".into(),
            Self::Float32 => "float".into(),
            Self::Float64 => "double".into(),
            Self::Unit => "void".into(),
            Self::Bool => "i1".into(),
            Self::String | Self::Symbol => "{ ptr, i64 }".into(),
            Self::Int(0, _) => "{}".into(),
            Self::Int(bits, _) => format!("i{bits}"),
        }
    }
    fn signed(&self) -> bool {
        matches!(self, Self::Int(_, true))
    }
}
// Never has no value representation. Only the machine return channel uses
// void; checked failures may return through that channel before propagation.
fn return_repr(graph: &iir::Package, ty: &Term, checked: bool) -> Result<Ty, String> {
    let ty = effects::row_result(ty).1;
    if *ty == Term::Primitive(P::Never) {
        Ok(Ty::Unit)
    } else {
        repr(graph, ty, checked)
    }
}
// Bound expansion as well as depth: a small DAG of nominal declarations can
// describe an exponentially large by-value physical tree.
const MAX_LAYOUT_NODES: usize = 65_536;

fn repr(graph: &iir::Package, ty: &Term, checked: bool) -> Result<Ty, String> {
    let mut remaining = MAX_LAYOUT_NODES;
    repr_at(graph, ty, 0, checked, &mut remaining)
}
fn repr_at(
    graph: &iir::Package,
    ty: &Term,
    depth: usize,
    checked: bool,
    remaining: &mut usize,
) -> Result<Ty, String> {
    *remaining = remaining
        .checked_sub(1)
        .ok_or("host representation node budget exhausted")?;
    if depth >= 128 {
        return Err("host representation nesting budget exhausted".into());
    }
    if graph
        .templates
        .iter()
        .any(|table| table.logical_type() == ty)
    {
        return Ok(Ty::TemplateIdentity(std::sync::Arc::new(ty.clone())));
    }
    match ty {
        Term::Optional(payload) => {
            let payload = if **payload == Term::Primitive(P::Never) {
                Ty::Unit
            } else {
                repr_at(graph, payload, depth + 1, checked, remaining)?
            };
            Ok(Ty::Optional(std::sync::Arc::new(payload)))
        }
        Term::ErrorResult { errors, result } => {
            let success = if **result == Term::Primitive(P::Never) {
                Ty::Unit
            } else {
                repr_at(graph, result, depth + 1, checked, remaining)?
            };
            let mut fields = vec![(None, success)];
            for error in errors {
                fields.push((
                    Some(error.clone()),
                    repr_at(graph, error, depth + 1, checked, remaining)?,
                ));
            }
            Ok(Ty::ErrorResult(std::sync::Arc::new(fields)))
        }
        Term::Record(fields) => Ok(Ty::Record(std::sync::Arc::new(
            fields
                .iter()
                .map(|(_, ty)| repr_at(graph, ty, depth + 1, checked, remaining))
                .collect::<Result<Vec<_>, _>>()?,
        ))),
        Term::Pointer(_) => {
            if !checked {
                return Err(
                    "host data pointers require paired Copy/Drop resource verification".into(),
                );
            }
            Ok(Ty::Pointer)
        }
        Term::Nominal { .. } => {
            if !checked {
                return Err(
                    "host nominal storage requires paired Copy/Drop resource verification".into(),
                );
            }
            let instance = graph
                .nominals
                .iter()
                .find(|instance| instance.ty == *ty)
                .ok_or("host nominal layout is not materialized")?;
            let definition = match &instance.definition {
                iir::NominalDefinition::Struct(definition) => definition,
                iir::NominalDefinition::Newtype(definition) => {
                    return repr_at(
                        graph,
                        &definition.representation,
                        depth + 1,
                        checked,
                        remaining,
                    );
                }
                iir::NominalDefinition::Enum(definition) => {
                    return Ok(Ty::Enum(std::sync::Arc::new(enums::EnumLayout::build(
                        graph, definition, depth, checked, remaining,
                    )?)));
                }
            };
            Ok(Ty::Record(std::sync::Arc::new(
                definition
                    .fields
                    .iter()
                    .map(|(_, ty)| repr_at(graph, ty, depth + 1, checked, remaining))
                    .collect::<Result<Vec<_>, _>>()?,
            )))
        }
        Term::Closure {
            environment,
            signature,
        } => {
            if repr_at(graph, signature, depth + 1, checked, remaining)? != Ty::FunctionPointer {
                return Err("host captured closure requires a runtime signature".into());
            }
            Ok(Ty::Closure(std::sync::Arc::new(
                environment
                    .iter()
                    .map(|ty| repr_at(graph, ty, depth + 1, checked, remaining))
                    .collect::<Result<Vec<_>, _>>()?,
            )))
        }
        Term::UnsafeFunction(signature) => repr_at(graph, signature, depth + 1, checked, remaining),
        Term::Function { parameters, .. } => {
            if parameters
                .iter()
                .any(|parameter| parameter.sort != Sort::Runtime)
            {
                return Err("host staged callable lacks an identity storage table".into());
            }
            Ok(Ty::FunctionPointer)
        }
        Term::Primitive(P::BitVec(width)) if *width <= bits::MAX_BIT_WIDTH as usize => {
            Ok(Ty::Int(*width as u32, false))
        }
        Term::Primitive(P::Void) => Ok(Ty::Unit),
        Term::Primitive(P::Bool) => Ok(Ty::Bool),
        Term::Primitive(P::Str) => Ok(Ty::String),
        Term::Primitive(P::Symbol) => Ok(Ty::Symbol),
        Term::Primitive(P::F16) => Ok(Ty::Float16),
        Term::Primitive(P::Bf16) => Ok(Ty::BFloat16),
        Term::Primitive(P::F32) => Ok(Ty::Float32),
        Term::Primitive(P::F64) => Ok(Ty::Float64),
        Term::Primitive(P::Char) => Ok(Ty::Int(32, false)),
        Term::Primitive(p) => graph
            .target
            .map_or_else(
                || p.fixed_integer_layout(),
                |target| target.integer_layout(*p),
            )
            .map(|(signed, bits)| Ty::Int(bits, signed))
            .ok_or_else(|| format!("host LLVM representation is not implemented for {ty}")),
        Term::Tuple(fields) => tuple_layout(graph, fields, depth, checked, remaining),
        Term::NormalForm { .. } => {
            let fields = graph
                .normal_forms
                .iter()
                .find_map(|head| head.tuple_elements(ty))
                .ok_or_else(|| format!("host has no trusted representation for {ty}"))?;
            tuple_layout(graph, fields, depth, checked, remaining)
        }
        _ => Err(format!(
            "host LLVM representation is not implemented for {ty}"
        )),
    }
}
fn tuple_layout(
    graph: &iir::Package,
    fields: &[Term],
    depth: usize,
    checked: bool,
    remaining: &mut usize,
) -> Result<Ty, String> {
    if fields.is_empty() {
        return Ok(Ty::Unit);
    }
    Ok(Ty::Record(std::sync::Arc::new(
        fields
            .iter()
            .map(|ty| repr_at(graph, ty, depth + 1, checked, remaining))
            .collect::<Result<Vec<_>, _>>()?,
    )))
}
#[derive(Clone)]
struct Value {
    ty: Ty,
    text: String,
}
impl Value {
    fn unit() -> Self {
        Self {
            ty: Ty::Unit,
            text: String::new(),
        }
    }
}
#[derive(Clone)]
struct Slot {
    ty: Ty,
    pointer: String,
}
struct Emitter<'a> {
    graph: &'a iir::Package,
    cleanup: Option<&'a iir::cleanup::FunctionSchedule>,
    schedules: Option<&'a iir::cleanup::Schedule>,
    resource_unwinds: Vec<frames::UnwindFrame<'a>>,
    checked: bool,
    function: Option<&'a Function>,
    closure_body: Option<nir::program::SourceRange>,
    failure_result: Option<Ty>,
    effects: Vec<std::sync::Arc<effects::DirectHandler>>,
    effect_resume: Option<(Slot, String, bool)>,
    native_return: Option<effects::ReturnStorage>,
    loop_destinations: BTreeSet<LocalId>,
    outlined_return: Option<effects::ReturnStorage>,
    outlined_loops: BTreeMap<LocalId, (String, String)>,
    lambdas: &'a BTreeMap<usize, usize>,
    template_identities: &'a callables::TemplateIdentities<'a>,
    locals: BTreeMap<LocalId, Slot>,
    live: BTreeMap<(LocalId, Vec<usize>), Slot>,
    temporaries: BTreeMap<(u32, u32), Slot>,
    temporary_live: BTreeMap<(u32, u32), Slot>,
    prepared: BTreeMap<(u32, u32, usize), cleanup::PreparedSlot>,
    receipt_paths: Vec<Vec<usize>>,
    pattern_orders: BTreeMap<(u32, u32), Slot>,
    temporary_fields: BTreeMap<((u32, u32), Vec<usize>), Slot>,
    loops: BTreeMap<LocalId, frames::LoopTarget>,
    prologue: String,
    body: String,
    declarations: &'a mut BTreeSet<String>,
    text_constants: &'a mut strings::TextConstants,
    serial: usize,
    terminated: bool,
    fuel: &'a mut usize,
}
impl Emitter<'_> {
    fn fresh(&mut self) -> String {
        self.serial += 1;
        format!("v{}", self.serial)
    }
    fn line(&mut self, s: impl AsRef<str>) {
        self.body.push_str("  ");
        self.body.push_str(s.as_ref());
        self.body.push('\n');
    }
    fn instruction(&mut self, ty: Ty, rhs: String) -> Value {
        let name = self.fresh();
        self.line(format!("%{name} = {rhs}"));
        Value {
            ty,
            text: format!("%{name}"),
        }
    }
    fn slot(&mut self, ty: Ty) -> Slot {
        let name = self.fresh();
        self.prologue.push_str(&format!(
            "  %{name} = alloca {}\n",
            if ty == Ty::Unit {
                "{}".into()
            } else {
                ty.llvm()
            }
        ));
        Slot {
            ty,
            pointer: format!("%{name}"),
        }
    }
    fn store(&mut self, slot: &Slot, value: &Value) -> Result<(), String> {
        if slot.ty != value.ty {
            return Err("host LLVM store representation mismatch".into());
        }
        if value.ty != Ty::Unit {
            self.line(format!(
                "store {} {}, ptr {}",
                value.ty.llvm(),
                value.text,
                slot.pointer
            ));
        }
        Ok(())
    }
    fn load(&mut self, slot: &Slot) -> Value {
        if slot.ty == Ty::Unit {
            Value::unit()
        } else {
            self.instruction(
                slot.ty.clone(),
                format!("load {}, ptr {}", slot.ty.llvm(), slot.pointer),
            )
        }
    }
    fn bind(&mut self, id: LocalId, value: &Value) -> Result<(), String> {
        let slot = self.slot(value.ty.clone());
        self.store(&slot, value)?;
        self.locals.insert(id, slot);
        self.set_live(id, &[], true)?;
        Ok(())
    }
    fn branch(&mut self, label: &str) {
        self.line(format!("br label %{label}"));
        self.terminated = true;
    }
    fn label(&mut self, label: &str) {
        self.body.push_str(&format!("{label}:\n"));
        self.terminated = false;
    }
    fn guard(&mut self, condition: &Value) -> Result<(), String> {
        self.guard_at(condition, None)
    }
    fn guard_at(
        &mut self,
        condition: &Value,
        predicate_source: Option<nir::program::SourceRange>,
    ) -> Result<(), String> {
        self.guard_with_cleanup(
            condition,
            predicate_source.map(|source| (source, iir::cleanup::Boundary::PredicateFailure)),
        )
    }
    fn guard_with_cleanup(
        &mut self,
        condition: &Value,
        cleanup: Option<(nir::program::SourceRange, iir::cleanup::Boundary)>,
    ) -> Result<(), String> {
        self.guard_with_diagnostic(condition, cleanup, None)
    }
    fn guard_with_diagnostic(
        &mut self,
        condition: &Value,
        cleanup: Option<(nir::program::SourceRange, iir::cleanup::Boundary)>,
        predicate: Option<nir::program::SourceRange>,
    ) -> Result<(), String> {
        if condition.ty != Ty::Bool {
            return Err("host predicate is not bool".into());
        }
        let good = self.fresh();
        let bad = self.fresh();
        self.line(format!(
            "br i1 {}, label %{good}, label %{bad}",
            condition.text
        ));
        self.label(&bad);
        if let Some(source) = predicate {
            self.report_predicate_failure(source)?;
        }
        if cleanup.is_some() && self.failure_result.is_some() {
            // A callee failure must not make successful cleanup calls appear to
            // fail. Restore the pending failure only after this frame unwinds.
            self.line("store i1 false, ptr @flurry_failed");
        }
        if let Some((source, boundary)) = cleanup {
            self.cleanup_boundary(source, boundary)?;
            self.unwind_resources()?;
        }
        if let Some(result) = self.failure_result.clone().filter(|_| cleanup.is_some()) {
            self.line("store i1 true, ptr @flurry_failed");
            if result == Ty::Unit {
                self.line("ret void");
            } else {
                self.line(format!("ret {} zeroinitializer", result.llvm()));
            }
        } else {
            self.line("call void @llvm.trap()");
            self.line("unreachable");
        }
        self.label(&good);
        Ok(())
    }
    fn report_predicate_failure(
        &mut self,
        source: nir::program::SourceRange,
    ) -> Result<(), String> {
        let source = source
            .original(&self.graph.expansion_sources)
            .ok_or("host predicate diagnostic has invalid source provenance")?;
        let owner = self
            .function
            .map(|function| function.declaration.to_string())
            .unwrap_or_else(|| "(closure)".into());
        let message = format!(
            "(runtime-check-failure (predicate {} {}) (owner {owner}))\n",
            source.start, source.end
        );
        *self.fuel = self
            .fuel
            .checked_sub(message.len().saturating_add(1))
            .ok_or("host predicate diagnostic budget exhausted")?;
        // dprintf writes to stderr without relying on stdio flush before trap.
        // Escape '%' even if future declaration displays introduce it.
        let format = message.replace('%', "%%");
        let bytes = format.as_bytes();
        let encoded = bytes
            .iter()
            .map(|byte| format!("\\{byte:02X}"))
            .collect::<String>();
        self.declarations
            .insert("declare i32 @dprintf(i32, ptr, ...)".into());
        let name = format!("flurry_predicate_message{}", self.declarations.len());
        self.declarations.insert(format!(
            "@{name} = private constant [{} x i8] c\"{encoded}\\00\"",
            bytes.len() + 1
        ));
        self.line(format!(
            "call i32 (i32, ptr, ...) @dprintf(i32 2, ptr @{name})"
        ));
        Ok(())
    }
    fn ret(
        &mut self,
        value: &Value,
        return_source: Option<nir::program::SourceRange>,
    ) -> Result<(), String> {
        if let (Some(storage), Some(source)) = (self.outlined_return.clone(), return_source) {
            self.cleanup_boundary(source, iir::cleanup::Boundary::TemporaryEnd)?;
            self.store(&storage.value, value)?;
            self.cleanup_boundary(source, iir::cleanup::Boundary::Return)?;
            self.unwind_effect_frames()?;
            self.line(format!(
                "store ptr {}, ptr @flurry_effect_control",
                storage.token
            ));
            return self.propagate_effect_control();
        }
        if let Some(source) = return_source {
            if self.cleanup.is_some_and(|scope| {
                matches!(
                    scope.context.last(),
                    Some(iir::cleanup::ContextKey::EffectHandler { .. })
                )
            }) {
                return self.return_from_handler(value, source);
            }
            self.cleanup_boundary(source, iir::cleanup::Boundary::TemporaryEnd)?;
        }
        self.emit_return(value, return_source, iir::cleanup::Boundary::Return)
    }

    fn emit_return(
        &mut self,
        value: &Value,
        return_source: Option<nir::program::SourceRange>,
        boundary: iir::cleanup::Boundary,
    ) -> Result<(), String> {
        if let Some(function) = self.function {
            let origin = return_source.unwrap_or(function.body.source);
            if let Some(binding) = function.result_binding {
                self.bind(binding, value)?;
            } else if !function.postconditions.is_empty() {
                self.retain_prepared_field(origin, 0, origin, value)?;
            }
            for condition in &function.postconditions {
                let checked = self.expr(condition, 0)?;
                let origin = return_source.unwrap_or(function.body.source);
                self.guard_with_diagnostic(
                    &checked,
                    Some((
                        condition.source,
                        iir::cleanup::Boundary::PostconditionFailure(origin),
                    )),
                    Some(condition.source),
                )?;
            }
            if function.result_binding.is_none() && !function.postconditions.is_empty() {
                self.finish_prepared_construction(origin)?;
            }
        }
        if let Some(source) = return_source {
            self.cleanup_boundary(source, boundary)?;
        } else if let Some(function) = self.function {
            self.cleanup_boundary(function.body.source, iir::cleanup::Boundary::FunctionEnd)?;
        } else if let Some(source) = self.closure_body {
            self.cleanup_boundary(source, iir::cleanup::Boundary::FunctionEnd)?;
        }
        self.line(if value.ty == Ty::Unit {
            "ret void".into()
        } else {
            format!("ret {} {}", value.ty.llvm(), value.text)
        });
        self.terminated = true;
        Ok(())
    }
    fn choice(
        &mut self,
        condition: &Value,
        yes: &Expr,
        no: Option<&Expr>,
        ty: Ty,
        depth: usize,
    ) -> Result<Value, String> {
        self.choice_with_effects(condition, yes, no, ty, depth, None)
    }
    fn choice_with_effects(
        &mut self,
        condition: &Value,
        yes: &Expr,
        no: Option<&Expr>,
        ty: Ty,
        depth: usize,
        selected: Option<&effects::Environment>,
    ) -> Result<Value, String> {
        if condition.ty != Ty::Bool {
            return Err("host branch condition is not bool".into());
        }
        let left = self.fresh();
        let right = self.fresh();
        let end = self.fresh();
        let result = self.slot(ty.clone());
        self.line(format!(
            "br i1 {}, label %{left}, label %{right}",
            condition.text
        ));
        let outer = self.locals.clone();
        self.label(&left);
        let v = self.expr_with_effects(yes, depth, selected)?;
        let a = !self.terminated;
        if a {
            self.store(&result, &v)?;
            self.branch(&end);
        }
        self.locals = outer.clone();
        self.label(&right);
        let v = if let Some(no) = no {
            self.expr_with_effects(no, depth, selected)?
        } else {
            Value::unit()
        };
        let b = !self.terminated;
        if b {
            self.store(&result, &v)?;
            self.branch(&end);
        }
        self.locals = outer;
        if !a && !b {
            return Ok(Value::unit());
        }
        self.label(&end);
        Ok(self.load(&result))
    }
    fn compare(&mut self, op: &str, a: Value, b: Value) -> Result<Value, String> {
        if a.ty == b.ty && matches!(a.ty, Ty::String | Ty::Symbol) {
            if !matches!(op, "eq" | "ne") {
                return Err("host strings and symbols only support equality".into());
            }
            let equal = self.string_equal(&a, &b);
            return Ok(if op == "eq" {
                equal
            } else {
                self.instruction(Ty::Bool, format!("xor i1 {}, true", equal.text))
            });
        }
        if a.ty != b.ty || !matches!(a.ty, Ty::Bool | Ty::Int(..) | Ty::FunctionPointer) {
            return Err("host scalar comparison representation mismatch".into());
        }
        if a.ty == Ty::Int(0, false) {
            return match op {
                "eq" | "ne" => Ok(Value {
                    ty: Ty::Bool,
                    text: (op == "eq").to_string(),
                }),
                _ => Err("host empty bit comparison only supports equality".into()),
            };
        }
        Ok(self.instruction(
            Ty::Bool,
            format!("icmp {op} {} {}, {}", a.ty.llvm(), a.text, b.text),
        ))
    }
    fn integer(&mut self, op: Op, a: Value, b: Value) -> Result<Value, String> {
        if a.ty != b.ty {
            return Err("host integer operand representation mismatch".into());
        }
        let Ty::Int(bits, signed) = a.ty else {
            return Err("host arithmetic requires a fixed integer".into());
        };
        let tag = if signed { "s" } else { "u" };
        match op {
            Op::Add | Op::Subtract | Op::Multiply => {
                let op = match op {
                    Op::Add => "add",
                    Op::Subtract => "sub",
                    _ => "mul",
                };
                let intrinsic = format!("llvm.{tag}{op}.with.overflow.i{bits}");
                let pair = format!("{{ i{bits}, i1 }}");
                self.declarations
                    .insert(format!("declare {pair} @{intrinsic}(i{bits}, i{bits})"));
                let raw = self.instruction(
                    a.ty.clone(),
                    format!(
                        "call {pair} @{intrinsic}(i{bits} {}, i{bits} {})",
                        a.text, b.text
                    ),
                );
                let overflow =
                    self.instruction(Ty::Bool, format!("extractvalue {pair} {}, 1", raw.text));
                let safe = self.instruction(Ty::Bool, format!("xor i1 {}, true", overflow.text));
                self.guard(&safe)?;
                Ok(self.instruction(a.ty.clone(), format!("extractvalue {pair} {}, 0", raw.text)))
            }
            Op::DivideTrunc | Op::RemainderTrunc => {
                let nonzero = self.compare(
                    "ne",
                    b.clone(),
                    Value {
                        ty: b.ty.clone(),
                        text: "0".into(),
                    },
                )?;
                self.guard(&nonzero)?;
                if signed {
                    let min = if bits == 128 {
                        i128::MIN.to_string()
                    } else {
                        (-(1i128 << (bits - 1))).to_string()
                    };
                    let is_min = self.compare(
                        "eq",
                        a.clone(),
                        Value {
                            ty: a.ty.clone(),
                            text: min,
                        },
                    )?;
                    let is_neg = self.compare(
                        "eq",
                        b.clone(),
                        Value {
                            ty: b.ty.clone(),
                            text: "-1".into(),
                        },
                    )?;
                    let bad = self
                        .instruction(Ty::Bool, format!("and i1 {}, {}", is_min.text, is_neg.text));
                    let good = self.instruction(Ty::Bool, format!("xor i1 {}, true", bad.text));
                    self.guard(&good)?;
                }
                let op = if op == Op::DivideTrunc { "div" } else { "rem" };
                Ok(self.instruction(
                    a.ty.clone(),
                    format!("{tag}{op} i{bits} {}, {}", a.text, b.text),
                ))
            }
            _ => {
                let predicate = match op {
                    Op::Equal => "eq".into(),
                    Op::NotEqual => "ne".into(),
                    Op::Less => format!("{tag}lt"),
                    Op::LessEqual => format!("{tag}le"),
                    Op::Greater => format!("{tag}gt"),
                    Op::GreaterEqual => format!("{tag}ge"),
                    _ => unreachable!(),
                };
                self.compare(&predicate, a, b)
            }
        }
    }
    fn expr(&mut self, expr: &Expr, depth: usize) -> Result<Value, String> {
        self.expr_with_effects(expr, depth, None)
    }
    fn expr_with_effects(
        &mut self,
        expr: &Expr,
        depth: usize,
        selected: Option<&effects::Environment>,
    ) -> Result<Value, String> {
        if self.terminated {
            return Ok(Value::unit());
        }
        if depth >= 128 || *self.fuel == 0 {
            return Err("host LLVM lowering budget exhausted".into());
        }
        *self.fuel -= 1;
        let result = self.inner(expr, depth + 1, selected);
        if result.is_ok() && !self.terminated {
            let transfers = self
                .cleanup
                .map(|s| {
                    s.transfers
                        .iter()
                        .filter(|t| t.source == expr.source)
                        .map(|t| (t.binding, t.fields.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for (binding, fields) in transfers {
                self.set_live(binding, &fields, false)?;
            }
        }
        if result.is_ok()
            && !self.terminated
            && !matches!(
                expr.kind,
                K::Block { .. } | K::CallPreparation { .. } | K::Return(_)
            )
            && !expr.shares_discard_temporary_boundary()
        {
            self.cleanup_boundary(expr.source, iir::cleanup::Boundary::TemporaryEnd)?;
        }
        result.map_err(|e| format!("{}..{}: {e}", expr.source.start, expr.source.end))
    }
    fn inner(
        &mut self,
        expr: &Expr,
        depth: usize,
        selected: Option<&effects::Environment>,
    ) -> Result<Value, String> {
        if selected.is_some()
            && !matches!(
                expr.kind,
                K::Call { .. }
                    | K::Block { .. }
                    | K::CallPreparation { .. }
                    | K::If { .. }
                    | K::Match { .. }
                    | K::EffectComputation { .. }
            )
        {
            return Err("host mapped computation requires an explicit execution boundary".into());
        }
        Ok(match &expr.kind {
            K::UnsafeScope(body) => self.expr(body, depth)?,
            K::Literal(literal) => {
                let ty = repr(self.graph, &expr.ty, self.checked)?;
                self.literal(literal, ty)?
            }
            K::EffectComputation { .. } if selected.is_some() => {
                let selected = selected.unwrap();
                *self.fuel = self
                    .fuel
                    .checked_sub(selected.len())
                    .ok_or("host mapped environment budget exhausted")?;
                let previous = std::mem::replace(&mut self.effects, selected.to_vec());
                let result = self.effect(expr, depth);
                self.effects = previous;
                result?
            }
            K::EffectResume(_)
            | K::EffectComputation { .. }
            | K::EffectRun { .. }
            | K::EffectHandle { .. }
            | K::EffectRequest { .. } => self.effect(expr, depth)?,
            K::Closure { .. } => self.closure(expr, depth)?,
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => {
                self.callable(expr)?
            }
            K::OptionalAbsent | K::OptionalPresent(_) => self.optional(expr, depth)?,
            K::Variant { index, fields, .. } => {
                self.variant(expr.source, *index, fields, &expr.ty, depth)?
            }
            K::Tuple(fields) => self.record(expr.source, fields, &expr.ty, depth)?,
            K::AddressOf(place) => self.address(place, depth)?,
            K::ReadPointer(pointer) => {
                let pointer = self.expr(pointer, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    let slot = self.pointer_slot(&pointer, &expr.ty)?;
                    self.load(&slot)
                }
            }
            K::PointerAnonymousField { pointer, index, .. }
            | K::PointerField { pointer, index, .. } => {
                let pointer_type = &pointer.ty;
                let pointer = self.expr(pointer, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    self.project_pointer(pointer, *index, pointers::pointee(pointer_type)?)?
                }
            }
            K::WritePointer { pointer, value } => {
                let pointer_type = &pointer.ty;
                let pointer = self.expr(pointer, depth)?;
                let value = self.expr(value, depth)?;
                if !self.terminated {
                    let slot = self.pointer_slot(&pointer, pointers::pointee(pointer_type)?)?;
                    self.store(&slot, &value)?;
                }
                Value::unit()
            }
            K::AnonymousRecord { fields } | K::Record { fields, .. } => {
                self.record(expr.source, fields, &expr.ty, depth)?
            }
            K::AnonymousField { value, index, .. } | K::Field { value, index, .. } => {
                let value = self.expr(value, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    self.record_field(&value, *index)?
                }
            }
            K::Unit => Value::unit(),
            K::Local(id) => {
                let slot = self
                    .locals
                    .get(id)
                    .cloned()
                    .ok_or("host local binding is unavailable")?;
                self.load(&slot)
            }
            K::Let { binding, value } => {
                let value = self.expr(value, depth)?;
                if !self.terminated {
                    self.bind(*binding, &value)?;
                }
                Value::unit()
            }
            K::ErrorConvert { conversion, value } => {
                let source_type = &value.ty;
                let value = self.expr(value, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    self.error_convert(*conversion, value, source_type, &expr.ty)?
                }
            }
            K::BitAssign {
                place,
                start,
                end,
                value,
                ..
            } => {
                let slot = self.assignment_slot(place)?;
                self.bit_write(
                    slot,
                    &[nir::program::BitSelection::Static {
                        start: *start,
                        end: *end,
                    }],
                    value,
                    depth,
                )?
            }
            K::BitUpdate {
                place,
                selections,
                value,
                ..
            } => {
                let slot = self.assignment_slot(place)?;
                self.bit_write(slot, selections, value, depth)?
            }
            K::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                let pointer_type = &pointer.ty;
                let pointer = self.expr(pointer, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    let slot = self.pointer_slot(&pointer, pointers::pointee(pointer_type)?)?;
                    self.bit_write(slot, selections, value, depth)?
                }
            }
            K::Assign { place, value } => {
                let value = self.expr(value, depth)?;
                if !self.terminated {
                    self.cleanup_boundary(expr.source, iir::cleanup::Boundary::Replacement)?;
                    let slot = self.assignment_slot(place)?;
                    self.store(&slot, &value)?;
                    self.set_live(
                        place.root,
                        &place
                            .fields
                            .iter()
                            .map(|(_, index)| *index)
                            .collect::<Vec<_>>(),
                        true,
                    )?;
                }
                Value::unit()
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let saved = self.locals.clone();
                for statement in statements {
                    self.expr(statement, depth)?;
                }
                let value = if let Some(result) = result {
                    self.expr_with_effects(result, depth, selected)?
                } else {
                    Value::unit()
                };
                if !self.terminated && matches!(expr.kind, K::Block { .. }) {
                    self.cleanup_boundary(expr.source, iir::cleanup::Boundary::ScopeEnd)?;
                }
                self.locals = saved;
                value
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => self.let_pattern(
                expr.source,
                pattern,
                value,
                guard.as_deref(),
                otherwise.as_deref(),
                depth,
            )?,
            K::Match { value, arms } => {
                let value = self.expr(value, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    self.match_with_fallback(value, arms, &expr.ty, depth, None, None, selected)?
                }
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = self.expr(condition, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    self.choice_with_effects(
                        &condition,
                        then_branch,
                        else_branch.as_deref(),
                        if expr.ty == Term::Primitive(P::Never) {
                            Ty::Unit
                        } else {
                            return_repr(self.graph, &expr.ty, self.checked)?
                        },
                        depth,
                        selected,
                    )?
                }
            }
            K::And(left, right) | K::Or(left, right) => {
                let left = self.expr(left, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                if matches!(left.ty, Ty::Int(_, _)) {
                    let right = self.expr(right, depth)?;
                    if self.terminated {
                        return Ok(Value::unit());
                    }
                    let op = if matches!(expr.kind, K::And(..)) {
                        "and"
                    } else {
                        "or"
                    };
                    return Ok(self.bit_binary(op, left, right)?);
                }
                let constant = Expr {
                    source: expr.source,
                    ty: Term::Primitive(P::Bool),
                    kind: K::Literal(Literal::Bool(matches!(expr.kind, K::Or(..)))),
                };
                if matches!(expr.kind, K::And(..)) {
                    self.choice(&left, right, Some(&constant), Ty::Bool, depth)?
                } else {
                    self.choice(&left, &constant, Some(right), Ty::Bool, depth)?
                }
            }
            K::Not(value) => {
                let value = self.expr(value, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                if value.ty == Ty::Int(0, false) {
                    return Ok(value);
                }
                let mask = if value.ty == Ty::Bool { "true" } else { "-1" };
                self.instruction(
                    value.ty.clone(),
                    format!("xor {} {}, {mask}", value.ty.llvm(), value.text),
                )
            }
            K::IntegerBinary { op, left, right } => {
                let left = self.expr(left, depth)?;
                let right = self.expr(right, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    self.integer(*op, left, right)?
                }
            }
            K::FloatNegate(value) => {
                let value = self.expr(value, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                self.instruction(
                    value.ty.clone(),
                    format!("fneg {} {}", value.ty.llvm(), value.text),
                )
            }
            K::Intrinsic { inputs, .. } => {
                let checked = nir::intrinsics::checked(expr).map_err(str::to_owned)?;
                let mut values = Vec::new();
                for input in inputs {
                    values.push(self.expr(input, depth)?);
                    if self.terminated {
                        return Ok(Value::unit());
                    }
                }
                let width = checked
                    .validate_target_layout(self.graph.target).map_err(str::to_owned)?
                    .ok_or("host intrinsic requires concrete width (platform integers need an explicit pointer-width)")?;
                let ty = repr(self.graph, &expr.ty, self.checked)?;
                if checked.operation().is_float_predicate() {
                    let input = &values[0];
                    if ty != Ty::Bool || !matches!(input.ty, Ty::Float32 | Ty::Float64) {
                        return Err("host float predicate representation mismatch".into());
                    }
                    if checked.operation() == nir::intrinsics::Operation::SignBit {
                        // Copy only the sign onto an exact, finite magnitude.
                        // Comparing the original value with zero loses -0 and NaNs.
                        let symbol = format!("llvm.copysign.f{width}");
                        let scalar = input.ty.llvm();
                        self.declarations
                            .insert(format!("declare {scalar} @{symbol}({scalar}, {scalar})"));
                        let sign = self.instruction(
                            input.ty.clone(),
                            format!(
                                "call {scalar} @{symbol}({scalar} 1.0, {scalar} {})",
                                input.text
                            ),
                        );
                        return Ok(self.instruction(
                            Ty::Bool,
                            format!("fcmp olt {scalar} {}, 0.0", sign.text),
                        ));
                    }
                    let mask = match checked.operation() {
                        nir::intrinsics::Operation::IsNan => 3,
                        nir::intrinsics::Operation::IsInfinite => 516,
                        nir::intrinsics::Operation::IsFinite => 504,
                        nir::intrinsics::Operation::IsNormal => 264,
                        nir::intrinsics::Operation::IsSubnormal => 144,
                        _ => unreachable!(),
                    };
                    let symbol = format!("llvm.is.fpclass.f{width}");
                    self.declarations.insert(format!(
                        "declare i1 @{symbol}({}, i32 immarg)",
                        input.ty.llvm()
                    ));
                    return Ok(self.instruction(
                        Ty::Bool,
                        format!(
                            "call i1 @{symbol}({} {}, i32 {mask})",
                            input.ty.llvm(),
                            input.text
                        ),
                    ));
                }
                if checked.operation().has_integer_exponent() {
                    if !matches!(ty, Ty::Float32 | Ty::Float64)
                        || values[0].ty != ty
                        || values[1].ty != Ty::Int(32, true)
                    {
                        return Err("host integer-exponent operand representation mismatch".into());
                    }
                    let scalar = ty.llvm();
                    let symbol = format!("llvm.{}.f{width}.i32", checked.operation().name());
                    self.declarations
                        .insert(format!("declare {scalar} @{symbol}({scalar}, i32)"));
                    return Ok(self.instruction(
                        ty,
                        format!(
                            "call {scalar} @{symbol}({scalar} {}, i32 {})",
                            values[0].text, values[1].text
                        ),
                    ));
                }
                if values.iter().any(|value| value.ty != ty) {
                    return Err("host intrinsic operand representation mismatch".into());
                }
                // These target operations lower to LLVM instructions, not to
                // fictitious llvm.bit_* functions. Both inputs above are
                // evaluated once in source order, even for zero/all-one masks.
                let bit_operation = match checked.operation() {
                    nir::intrinsics::Operation::BitAnd => Some("and"),
                    nir::intrinsics::Operation::BitOr => Some("or"),
                    nir::intrinsics::Operation::BitXor => Some("xor"),
                    _ => None,
                };
                if let Some(operation) = bit_operation {
                    return Ok(self.instruction(
                        ty.clone(),
                        format!(
                            "{operation} {} {}, {}",
                            ty.llvm(),
                            values[0].text,
                            values[1].text
                        ),
                    ));
                }
                // LLVM 21's native minimumNumber lowering can lose signed-zero
                // ordering. Use the propagating intrinsic plus explicit single-NaN
                // selection; both inputs above are still evaluated exactly once.
                let number_operation = matches!(
                    checked.operation(),
                    nir::intrinsics::Operation::MinimumNumber
                        | nir::intrinsics::Operation::MaximumNumber
                );
                let operation = match checked.operation() {
                    nir::intrinsics::Operation::SaturatingAdd => {
                        if matches!(ty, Ty::Int(_, true)) {
                            "sadd.sat"
                        } else {
                            "uadd.sat"
                        }
                    }
                    nir::intrinsics::Operation::SaturatingSubtract => {
                        if matches!(ty, Ty::Int(_, true)) {
                            "ssub.sat"
                        } else {
                            "usub.sat"
                        }
                    }
                    nir::intrinsics::Operation::IntegerMinimum => {
                        if matches!(ty, Ty::Int(_, true)) {
                            "smin"
                        } else {
                            "umin"
                        }
                    }
                    nir::intrinsics::Operation::IntegerMaximum => {
                        if matches!(ty, Ty::Int(_, true)) {
                            "smax"
                        } else {
                            "umax"
                        }
                    }
                    nir::intrinsics::Operation::MinimumNumber => "minimum",
                    nir::intrinsics::Operation::MaximumNumber => "maximum",
                    operation => operation.name(),
                };
                let suffix = match &ty {
                    Ty::Float32 | Ty::Float64 => format!("f{width}"),
                    Ty::Int(bits, _) if *bits == width => format!("i{width}"),
                    _ => return Err("host intrinsic scalar representation mismatch".into()),
                };
                let symbol = format!("llvm.{operation}.{suffix}");
                let total_integer = matches!(
                    checked.operation(),
                    nir::intrinsics::Operation::LeadingZeros
                        | nir::intrinsics::Operation::TrailingZeros
                        | nir::intrinsics::Operation::WrappingAbsoluteValue
                );
                let mut parameters =
                    vec![ty.llvm(); checked.operation().operand_count()].join(", ");
                if total_integer {
                    parameters.push_str(", i1 immarg");
                }
                self.declarations
                    .insert(format!("declare {} @{symbol}({parameters})", ty.llvm()));
                let mut arguments = values
                    .iter()
                    .map(|v| format!("{} {}", ty.llvm(), v.text))
                    .collect::<Vec<_>>()
                    .join(", ");
                // Zero counts and wrapping absolute value are total, including
                // zero and INT_MIN. The poison flag is a lowering constant,
                // never an extra source argument.
                if total_integer {
                    arguments.push_str(", i1 false");
                }
                let reduced = self.instruction(
                    ty.clone(),
                    format!("call {} @{symbol}({arguments})", ty.llvm()),
                );
                if number_operation {
                    let a = &values[0];
                    let b = &values[1];
                    let a_nan = self.instruction(
                        Ty::Bool,
                        format!("fcmp uno {} {}, {}", ty.llvm(), a.text, a.text),
                    );
                    let b_nan = self.instruction(
                        Ty::Bool,
                        format!("fcmp uno {} {}, {}", ty.llvm(), b.text, b.text),
                    );
                    let single_nan = self
                        .instruction(Ty::Bool, format!("xor i1 {}, {}", a_nan.text, b_nan.text));
                    let numeric = self.instruction(
                        ty.clone(),
                        format!(
                            "select i1 {}, {} {}, {} {}",
                            a_nan.text,
                            ty.llvm(),
                            b.text,
                            ty.llvm(),
                            a.text
                        ),
                    );
                    self.instruction(
                        ty.clone(),
                        format!(
                            "select i1 {}, {} {}, {} {}",
                            single_nan.text,
                            ty.llvm(),
                            numeric.text,
                            ty.llvm(),
                            reduced.text
                        ),
                    )
                } else {
                    reduced
                }
            }
            K::FloatBinary { op, left, right } => {
                let a = self.expr(left, depth)?;
                let b = self.expr(right, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                if a.ty != b.ty
                    || !matches!(
                        a.ty.clone(),
                        Ty::Float16 | Ty::BFloat16 | Ty::Float32 | Ty::Float64
                    )
                {
                    return Err("host floating operand mismatch".into());
                }
                let operation = match op {
                    nir::program::FloatOp::Add => "fadd",
                    nir::program::FloatOp::Subtract => "fsub",
                    nir::program::FloatOp::Multiply => "fmul",
                    nir::program::FloatOp::Divide => "fdiv",
                };
                let value = self.instruction(
                    a.ty.clone(),
                    format!("{operation} {} {}, {}", a.ty.llvm(), a.text, b.text),
                );
                // Match the reference evaluator's deterministic arithmetic NaN;
                // literal bits and unary sign changes retain their payloads.
                let nan = self.instruction(
                    Ty::Bool,
                    format!(
                        "fcmp uno {} {}, {}",
                        value.ty.llvm(),
                        value.text,
                        value.text
                    ),
                );
                let canonical = match a.ty {
                    Ty::Float16 => "bitcast (i16 32256 to half)",
                    Ty::BFloat16 => "bitcast (i16 32704 to bfloat)",
                    Ty::Float32 => "bitcast (i32 2143289344 to float)",
                    _ => "bitcast (i64 9221120237041090560 to double)",
                };
                self.instruction(
                    a.ty.clone(),
                    format!(
                        "select i1 {}, {} {canonical}, {} {}",
                        nan.text,
                        a.ty.llvm(),
                        a.ty.llvm(),
                        value.text
                    ),
                )
            }
            K::IntegerNegate(value) => {
                let value = self.expr(value, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                self.integer(
                    Op::Subtract,
                    Value {
                        ty: value.ty.clone(),
                        text: "0".into(),
                    },
                    value,
                )?
            }
            K::TemplateEqual(a, b) => {
                let a = self.expr(a, depth)?;
                let b = self.expr(b, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                if a.ty != b.ty || !matches!(a.ty, Ty::TemplateIdentity(_)) {
                    return Err(
                        "host template comparison requires matching identity domains".into(),
                    );
                }
                self.instruction(Ty::Bool, format!("icmp eq i64 {}, {}", a.text, b.text))
            }
            K::SymbolName(operand) => {
                let mut value = self.expr(operand, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                if value.ty != Ty::Symbol {
                    return Err("host symbol name requires a Symbol".into());
                }
                value.ty = Ty::String;
                value
            }
            K::EnumName(value) => self.enum_name(value, depth)?,
            K::TextQuery { subject, query } => self.text_query(expr, subject, *query, depth)?,
            K::ScalarEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b) => {
                let a = self.expr(a, depth)?;
                let b = self.expr(b, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                if matches!(
                    a.ty.clone(),
                    Ty::Float16 | Ty::BFloat16 | Ty::Float32 | Ty::Float64
                ) {
                    if a.ty != b.ty {
                        return Err("host floating comparison mismatch".into());
                    }
                    let predicate = match expr.kind {
                        K::ScalarEqual(..) => "oeq",
                        K::ScalarLess(..) => "olt",
                        K::ScalarLessEqual(..) => "ole",
                        K::ScalarGreater(..) => "ogt",
                        _ => "oge",
                    };
                    return Ok(self.instruction(
                        Ty::Bool,
                        format!("fcmp {predicate} {} {}, {}", a.ty.llvm(), a.text, b.text),
                    ));
                }
                let tag = if a.ty.signed() { "s" } else { "u" };
                let op = match expr.kind {
                    K::ScalarEqual(..) => "eq".into(),
                    K::ScalarLess(..) => format!("{tag}lt"),
                    K::ScalarLessEqual(..) => format!("{tag}le"),
                    K::ScalarGreater(..) => format!("{tag}gt"),
                    _ => format!("{tag}ge"),
                };
                self.compare(&op, a, b)?
            }
            K::Loop { identity, body } => {
                let head = self.fresh();
                let end = self.fresh();
                self.branch(&head);
                self.label(&head);
                let control = if self.checked && self.loop_destinations.contains(identity) {
                    Some((self.slot(Ty::Bool).pointer, self.slot(Ty::Bool).pointer))
                } else {
                    None
                };
                self.loops.insert(
                    *identity,
                    frames::LoopTarget {
                        control,
                        head: head.clone(),
                        end: end.clone(),
                        owner: self
                            .cleanup
                            .map(|scope| (scope.instance.clone(), scope.context.clone())),
                    },
                );
                self.expr(body, depth)?;
                if !self.terminated {
                    self.branch(&head);
                }
                self.loops.remove(identity);
                self.label(&end);
                if expr.ty == Term::Primitive(P::Never) {
                    self.line("unreachable");
                    self.terminated = true;
                }
                Value::unit()
            }
            K::Break(id) | K::Continue(id) => {
                if !self.loops.contains_key(id) {
                    if let Some((exit, again)) = self.outlined_loops.get(id).cloned() {
                        let is_break = matches!(expr.kind, K::Break(_));
                        self.cleanup_boundary(
                            expr.source,
                            if is_break {
                                iir::cleanup::Boundary::Break
                            } else {
                                iir::cleanup::Boundary::Continue
                            },
                        )?;
                        self.unwind_effect_frames()?;
                        self.line(format!(
                            "store ptr {}, ptr @flurry_effect_control",
                            if is_break { exit } else { again }
                        ));
                        self.propagate_effect_control()?;
                        return Ok(Value::unit());
                    }
                }
                let target = self
                    .loops
                    .get(id)
                    .cloned()
                    .ok_or("host loop target is unavailable")?;
                if !target.belongs_to(self.cleanup) {
                    self.loop_from_handler(
                        *id,
                        &target,
                        expr.source,
                        matches!(expr.kind, K::Break(_)),
                    )?;
                    return Ok(Value::unit());
                }
                let boundary = if matches!(expr.kind, K::Break(_)) {
                    iir::cleanup::Boundary::Break
                } else {
                    iir::cleanup::Boundary::Continue
                };
                self.cleanup_boundary(expr.source, boundary)?;
                self.branch(if matches!(expr.kind, K::Break(_)) {
                    &target.end
                } else {
                    &target.head
                });
                Value::unit()
            }
            K::Return(value) => {
                let value = if let Some(value) = value {
                    self.expr(value, depth)?
                } else {
                    Value::unit()
                };
                if !self.terminated {
                    self.ret(&value, Some(expr.source))?;
                }
                Value::unit()
            }
            K::RequirePredicate(e) | K::EnsurePredicate(e) | K::CheckPredicate(e) => {
                let condition = self.expr(e, depth)?;
                if !self.terminated {
                    self.guard_with_diagnostic(
                        &condition,
                        Some((e.source, iir::cleanup::Boundary::PredicateFailure)),
                        Some(e.source),
                    )?;
                }
                Value::unit()
            }
            K::Discard(e) => {
                let value = self.expr(e, depth)?;
                if !self.terminated {
                    self.cleanup_discard(expr.source, e.source, &value)?;
                }
                Value::unit()
            }
            K::ComptimeIntegerConvert(_) => {
                return Err(
                    "comptime integer conversion must complete before LLVM lowering".into(),
                );
            }
            K::IntegerRepresentation(e) => {
                if !nir::program::integer_representation_valid(&e.ty, &expr.ty) {
                    return Err("host integer representation domain mismatch".into());
                }
                let value = self.expr(e, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    let Ty::Int(width, _) = value.ty else {
                        return Err("host integer representation requires an integer".into());
                    };
                    if width == 128 {
                        Value {
                            ty: Ty::Int(128, false),
                            text: value.text,
                        }
                    } else {
                        self.instruction(
                            Ty::Int(128, false),
                            format!("zext {} {} to i128", value.ty.llvm(), value.text),
                        )
                    }
                }
            }
            K::TakeBits(e) => {
                let value = self.expr(e, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    self.to_bits(value)?
                }
            }
            K::BitEqual(a, b) | K::BitConcat(a, b) => {
                let a = self.expr(a, depth)?;
                let b = self.expr(b, depth)?;
                if self.terminated {
                    Value::unit()
                } else if matches!(expr.kind, K::BitEqual(..)) {
                    self.bit_binary("icmp eq", a, b)?
                } else {
                    self.bit_concat(a, b)?
                }
            }
            K::BitSlice { value, start, end } => {
                let value = self.expr(value, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    self.bit_slice(value, *start, *end)?
                }
            }
            K::BitIndex(value, index) => {
                let value = self.expr(value, depth)?;
                let index = self.expr(index, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    self.bit_window(value, index, None, 1)?
                }
            }
            K::BitWindow { value, start, end } => {
                let value = self.expr(value, depth)?;
                let start = self.expr(start, depth)?;
                let end = self.expr(end, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    let target = repr(self.graph, &expr.ty, self.checked)?;
                    let width = bit_width(&target).ok_or("host bit window result has no width")?;
                    self.bit_window(value, start, Some(end), width)?
                }
            }
            K::BitConvert(e) => {
                let value = self.expr(e, depth)?;
                if self.terminated {
                    Value::unit()
                } else {
                    self.bit_convert(value, repr(self.graph, &expr.ty, self.checked)?)?
                }
            }
            K::Snapshot(e) => self.expr(e, depth)?,
            K::NewtypeConvert(value) => {
                let value = self.expr(value, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                // IIR has validated exactly one declared representation edge.
                // Equal LLVM layouts do not authorize any other conversion.
                let target = repr(self.graph, &expr.ty, self.checked)?;
                if value.ty != target {
                    return Err("host newtype representation edge changes its layout".into());
                }
                value
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                // The target expression runs once, before any argument effects.
                let function = self.expr(callee, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                if matches!(function.ty, Ty::Closure(_)) {
                    if self.checked && !self.has_checked_closure_call(expr.source)? {
                        // This invocation has no acquired resource frame within
                        // an otherwise checked parent. Keep the syntactic arm,
                        // but never execute its callback without that evidence.
                        // In particular, do not borrow another call's frame or
                        // fall back to the canonical unchecked lambda body.
                        self.line("call void @llvm.trap()");
                        self.line("unreachable");
                        self.terminated = true;
                        return Ok(Value::unit());
                    }
                    self.check_closure_receiver(callee, arguments)?;
                }
                let (function, environment) = self.closure_callee(callee, function)?;
                let function = self.specialize_closure_call(expr.source, function)?;
                let function = self.specialize_named_call(expr.source, function)?;
                let Term::Function { parameters, result } = callee.ty.callable_signature() else {
                    return Err("host callee has no concrete function signature".into());
                };
                let mut values = vec![None; parameters.len()];
                let caller = self.locals.clone();
                let mut frame = BTreeMap::new();
                for argument in arguments {
                    self.locals = caller.clone();
                    if argument.is_default {
                        self.locals.extend(frame.clone());
                    }
                    let value = if argument.is_default {
                        self.eval_default(
                            expr.source,
                            argument,
                            frame.clone(),
                            caller.clone(),
                            depth,
                        )?
                    } else {
                        self.expr(&argument.value, depth)?
                    };
                    if self.terminated {
                        self.locals = caller;
                        return Ok(Value::unit());
                    }
                    self.retain_prepared_field(
                        expr.source,
                        argument.parameter,
                        argument.value.source,
                        &value,
                    )?;
                    let cell = values
                        .get_mut(argument.parameter)
                        .ok_or("host call slot out of range")?;
                    if cell.is_some() {
                        return Err("host duplicate call slot".into());
                    }
                    if let Some(binding) = parameter_bindings.get(argument.parameter) {
                        let slot = self.slot(value.ty.clone());
                        self.store(&slot, &value)?;
                        frame.insert(*binding, slot);
                    }
                    *cell = Some(value);
                }
                self.locals = caller;
                let method_frame = self.method_entry(callee, &values, expr.source, depth)?;
                let mut args = environment
                    .into_iter()
                    .map(|slot| format!("ptr {}", slot.pointer))
                    .collect::<Vec<_>>();
                for (value, parameter) in values.into_iter().zip(parameters) {
                    let value = value.ok_or("host missing call slot")?;
                    if parameter.sort != Sort::Runtime
                        || value.ty != repr(self.graph, &parameter.ty, self.checked)?
                    {
                        return Err("host call input representation mismatch".into());
                    }
                    if value.ty != Ty::Unit {
                        args.push(format!("{} {}", value.ty.llvm(), value.text));
                    }
                }
                let (row, logical_result) = effects::row_result(result);
                args.extend(self.effect_arguments(row, selected)?);
                let ty = return_repr(self.graph, result, self.checked)?;
                let call = format!("call {} {}({})", ty.llvm(), function.text, args.join(", "));
                self.finish_prepared_construction(expr.source)?;
                let value = if ty == Ty::Unit {
                    self.line(call);
                    Value::unit()
                } else {
                    self.instruction(ty, call)
                };
                if self.failure_result.is_some() {
                    let failed = self.instruction(Ty::Bool, "load i1, ptr @flurry_failed".into());
                    let succeeded =
                        self.instruction(Ty::Bool, format!("xor i1 {}, true", failed.text));
                    self.guard_at(&succeeded, Some(expr.source))?;
                }
                if !row.is_empty() {
                    self.check_effect_control(expr.source)?;
                }
                if *logical_result == Term::Primitive(P::Never) {
                    self.line("unreachable");
                    self.terminated = true;
                } else {
                    self.method_exit(method_frame, &value, depth)?;
                }
                value
            }
            _ => {
                return Err(format!(
                    "host LLVM lowering is not implemented at {}..{} for {}",
                    expr.source.start, expr.source.end, expr.kind
                ));
            }
        })
    }
}

pub(super) fn emit(
    graph: &iir::Package,
    mut fuel: usize,
    checked: bool,
    schedule: Option<&iir::cleanup::Schedule>,
    entry: Option<&iir::InstanceKey>,
) -> Result<String, String> {
    if graph.roots.is_empty() {
        return Err("host pipeline requires one entry".into());
    }
    let root = graph
        .functions
        .iter()
        .position(|f| &f.key == entry.unwrap_or(&graph.roots[0]))
        .ok_or("host entry missing")?;
    let contracts = iir::effects::Contracts::acquire(graph, &mut fuel)?;
    let mut declarations = BTreeSet::new();
    let mut text_constants = strings::TextConstants::default();
    let mut functions = String::new();
    let mut lambdas = BTreeMap::new();
    let mut closure_nodes = Vec::new();
    let mut template_identities = callables::TemplateIdentities::new(graph.templates.len());
    for instance in &graph.functions {
        nir::visit::expressions(&instance.function, &mut fuel, |expr, fuel| {
            template_identities.observe(graph, expr, fuel)?;
            if let K::Closure { .. } = &expr.kind {
                let address = expr as *const Expr as usize;
                if !lambdas.contains_key(&address) {
                    lambdas.insert(address, closure_nodes.len());
                    closure_nodes.push(expr);
                }
            }
            Ok(())
        })?;
    }
    // Addresses are only lookup keys during this invocation. Symbol numbers follow
    // deterministic traversal order, never pointer ordering or allocation addresses.
    let mut bodies = Vec::new();
    for (index, instance) in graph.functions.iter().enumerate() {
        let f = &instance.function;
        bodies.push((
            format!("flurry_{index}"),
            &f.signature,
            f.parameters.as_slice(),
            &f.body,
            Some(f),
            None,
            None,
        ));
        if let Some(schedule) = schedule {
            let scopes = cleanup::schedules_for(schedule, &instance.key, &mut fuel)?;
            let canonical = cleanup::canonical_scope(&scopes).map(|(index, _)| index);
            for (scope_index, scope) in scopes {
                if Some(scope_index) == canonical {
                    continue;
                }
                bodies.push((
                    format!("flurry_{index}_scope_{scope_index}"),
                    &f.signature,
                    f.parameters.as_slice(),
                    &f.body,
                    Some(f),
                    None,
                    Some(scope),
                ));
            }
        }
    }
    for (index, node) in closure_nodes.into_iter().enumerate() {
        let K::Closure {
            parameters, body, ..
        } = &node.kind
        else {
            unreachable!()
        };
        bodies.push((
            format!("flurry_lambda_{index}"),
            node.ty.callable_signature(),
            parameters.as_slice(),
            body.as_ref(),
            None,
            Some(node),
            None,
        ));
        if let Some(schedule) = schedule {
            for (scope_index, scope) in closures::schedules_for(graph, schedule, node, &mut fuel)? {
                bodies.push((
                    format!("flurry_lambda_{index}_scope_{scope_index}"),
                    node.ty.callable_signature(),
                    parameters.as_slice(),
                    body.as_ref(),
                    None,
                    Some(node),
                    Some(scope),
                ));
            }
        }
    }
    for (symbol, signature, bindings, body, metadata, closure, closure_scope) in bodies {
        let Term::Function { parameters, result } = signature else {
            return Err("host function signature is not concrete".into());
        };
        let (effect_row, logical_result) = effects::row_result(result);
        let never_returns = *logical_result == Term::Primitive(P::Never);
        let result = return_repr(graph, result, checked)?;
        if bindings.len() != parameters.len() {
            return Err("host parameter frame mismatch".into());
        }
        let mut e = Emitter {
            graph,
            cleanup: if let (Some(f), Some(schedule)) = (metadata, schedule) {
                let instance = graph
                    .functions
                    .iter()
                    .find(|instance| std::ptr::eq(&instance.function, f))
                    .ok_or("host cleanup function identity missing")?;
                if let Some(scope) = closure_scope {
                    Some(scope)
                } else {
                    let scopes = cleanup::schedules_for(schedule, &instance.key, &mut fuel)?;
                    cleanup::canonical_scope(&scopes).map(|(_, scope)| scope)
                }
            } else if let (Some(node), Some(schedule)) = (closure, schedule) {
                if let Some(scope) = closure_scope {
                    Some(scope)
                } else {
                    closures::schedules_for(graph, schedule, node, &mut fuel)?
                        .first()
                        .map(|(_, scope)| *scope)
                }
            } else {
                None
            },
            schedules: schedule,
            resource_unwinds: vec![],
            checked,
            function: metadata,
            closure_body: closure.map(|_| body.source),
            failure_result: checked.then(|| result.clone()),
            effects: vec![],
            effect_resume: None,
            native_return: None,
            loop_destinations: Default::default(),
            outlined_return: None,
            outlined_loops: BTreeMap::new(),
            lambdas: &lambdas,
            template_identities: &template_identities,
            locals: BTreeMap::new(),
            live: BTreeMap::new(),
            temporaries: BTreeMap::new(),
            temporary_live: BTreeMap::new(),
            prepared: BTreeMap::new(),
            receipt_paths: vec![],
            pattern_orders: BTreeMap::new(),
            temporary_fields: BTreeMap::new(),
            loops: BTreeMap::new(),
            prologue: String::new(),
            body: String::new(),
            declarations: &mut declarations,
            text_constants: &mut text_constants,
            serial: 0,
            terminated: false,
            fuel: &mut fuel,
        };
        // Every emitted checked named body needs its acquired resource frame.
        // Context-dependent functions may be materialized by taking their
        // address without ever acquiring a call-specific cleanup schedule.
        // Missing evidence is not an empty (resource-free) cleanup program.
        if checked && metadata.is_some() && e.cleanup.is_none() {
            return Err(format!(
                "host checked body {symbol} has no acquired cleanup schedule"
            ));
        }
        if checked {
            let destinations = effects::control_destinations(body, e.fuel)?;
            e.loop_destinations = destinations.loops;
            if destinations.returns {
                let token = e.slot(Ty::Bool).pointer;
                let value = e.slot(result.clone());
                e.native_return = Some(effects::ReturnStorage { token, value });
            }
        }
        let mut inputs = Vec::new();
        if let Some(node) = closure {
            e.bind_environment(node, &mut inputs)?;
        }
        for (i, (id, p)) in bindings.iter().zip(parameters).enumerate() {
            if p.sort != Sort::Runtime {
                return Err("host function retains staged inputs".into());
            }
            let ty = repr(graph, &p.ty, checked)?;
            if ty != Ty::Unit {
                inputs.push(format!("{} %arg{i}", ty.llvm()));
            }
            e.bind(
                *id,
                &Value {
                    ty,
                    text: format!("%arg{i}"),
                },
            )?;
        }
        e.bind_effect_parameters(&contracts, effect_row, &mut inputs)?;
        if let Some(f) = metadata {
            for snapshot in &f.entry_snapshots {
                let slot = e
                    .locals
                    .get(&snapshot.parameter)
                    .cloned()
                    .ok_or("host entry snapshot target missing")?;
                let value = e.load(&slot);
                e.bind(snapshot.binding, &value)?;
            }
            for predicate in &f.preconditions {
                let value = e.expr(predicate, 0)?;
                e.guard_with_diagnostic(
                    &value,
                    Some((predicate.source, iir::cleanup::Boundary::PredicateFailure)),
                    Some(predicate.source),
                )?;
            }
        }
        let value = e.expr(body, 0)?;
        if !e.terminated {
            if never_returns {
                return Err("host Never function has a normal completion path".into());
            }
            if value.ty != result {
                return Err("host return representation mismatch".into());
            }
            e.ret(&value, None)?;
        }
        functions.push_str(&format!(
            "define internal {} @{symbol}({}) {{\nentry:\n{}{}}}\n\n",
            result.llvm(),
            inputs.join(", "),
            e.prologue,
            e.body
        ));
    }
    let Term::Function { parameters, result } = &graph.functions[root].function.signature else {
        unreachable!()
    };
    if !parameters.is_empty() || !effects::row_result(result).0.is_empty() {
        return Err("host entry requires a no-argument, effect-free wrapper".into());
    }
    let never_returns = **result == Term::Primitive(P::Never);
    let logical_result = result;
    let result = return_repr(graph, result, checked)?;
    // A transparent machine layout does not erase a nominal result's
    // ownership. In particular, newtypes can lower to scalar, string or unit
    // storage and otherwise bypass the structural printer's admission check.
    if matches!(logical_result.as_ref(), Term::Nominal { .. }) {
        return Err("owned nominal values need an entry cleanup adapter; use --discard-result with Copy/Drop protocols".into());
    }
    let mut module = String::from(
        "; Flurry native host module\nsource_filename = \"flurry-host\"\ndeclare void @llvm.trap()\ndeclare i32 @printf(ptr, ...)\n",
    );
    module.push_str(include_str!("output.ll"));
    if let Some(target) = graph.target {
        module.push_str(&format!(
            "!flurry.target = !{{!0}}\n!0 = !{{i32 {}}}\n",
            target.pointer_bits()
        ));
    }
    if checked {
        module.push_str("@flurry_failed = internal thread_local global i1 false\n@flurry_effect_control = internal thread_local global ptr null\n\n");
        module.push_str("define internal void @flurry_check_failure() {\nentry:\n  %failed = load i1, ptr @flurry_failed\n  br i1 %failed, label %bad, label %good\nbad:\n  call void @llvm.trap()\n  unreachable\ngood:\n  ret void\n}\n\n");
        let (call, ret) = if never_returns {
            (format!("call void @flurry_{root}()"), "unreachable".into())
        } else if result == Ty::Unit {
            (format!("call void @flurry_{root}()"), "ret void".into())
        } else {
            (
                format!("%value = call {} @flurry_{root}()", result.llvm()),
                format!("ret {} %value", result.llvm()),
            )
        };
        module.push_str(&format!("define internal {} @flurry_entry() {{\nentry:\n  %outer_return = load ptr, ptr @flurry_effect_control\n  store ptr null, ptr @flurry_effect_control\n  store i1 false, ptr @flurry_failed\n  {call}\n  call void @flurry_check_failure()\n  %pending_return = load ptr, ptr @flurry_effect_control\n  %settled = icmp eq ptr %pending_return, null\n  br i1 %settled, label %complete, label %unhandled\nunhandled:\n  call void @llvm.trap()\n  unreachable\ncomplete:\n  store ptr %outer_return, ptr @flurry_effect_control\n  {ret}\n}}\n\n", result.llvm()));
    }
    for declaration in declarations {
        module.push_str(&declaration);
        module.push('\n');
    }
    if never_returns {
        module.push_str(&functions);
        let target = if checked {
            "flurry_entry".to_string()
        } else {
            format!("flurry_{root}")
        };
        module.push_str(&format!(
            "define i32 @main() {{\nentry:\n  call void @{target}()\n  unreachable\n}}\n"
        ));
        return Ok(module);
    }
    if matches!(result, Ty::Record(_) | Ty::ErrorResult(_))
        || matches!(logical_result.as_ref(), Term::Tuple(_))
        || graph
            .normal_forms
            .iter()
            .any(|head| head.tuple_elements(logical_result).is_some())
    {
        module.push_str(&functions);
        let target = if checked {
            "flurry_entry".to_string()
        } else {
            format!("flurry_{root}")
        };
        module.push_str(&entry::product_main(
            graph,
            logical_result,
            &result,
            &target,
            checked,
            &mut fuel,
        )?);
        return Ok(module);
    }
    if matches!(result, Ty::String | Ty::Symbol) {
        let printer = if result == Ty::Symbol {
            "flurry_print_symbol"
        } else {
            "flurry_print_string"
        };
        module.push_str(&functions);
        module.push_str(include_str!("print_string.ll"));
        let target = if checked {
            "flurry_entry".to_string()
        } else {
            format!("flurry_{root}")
        };
        module.push_str(&format!("define i32 @main() {{\nentry:\n  %value = call {{ ptr, i64 }} @{target}()\n  call void @{printer}({{ ptr, i64 }} %value)\n  call i32 @flurry_output_char(i32 10)\n  %status = call i32 @flurry_output_finish()\n  ret i32 %status\n}}\n"));
        return Ok(module);
    }
    let (format, body) = match result {
        Ty::Unit => (
            "(unit)\n",
            format!(
                "  call void @flurry_{root}()\n  %output_write = call i32 (ptr, ...) @printf(ptr @result_format)\n"
            ),
        ),
        Ty::Bool => {
            module.push_str("@bool_true = private constant [5 x i8] c\"true\\00\"\n@bool_false = private constant [6 x i8] c\"false\\00\"\n");
            (
                "(bool %s)\n",
                format!(
                    "  %value = call i1 @flurry_{root}()\n  %text = select i1 %value, ptr @bool_true, ptr @bool_false\n  %output_write = call i32 (ptr, ...) @printf(ptr @result_format, ptr %text)\n"
                ),
            )
        }
        Ty::Float16 | Ty::BFloat16 => {
            let format = if result == Ty::Float16 {
                "(float16 (bits %04x))\n"
            } else {
                "(bfloat16 (bits %04x))\n"
            };
            (
                format,
                format!(
                    "  %value = call {} @flurry_{root}()\n  %bits = bitcast {} %value to i16\n  %wide = zext i16 %bits to i32\n  %output_write = call i32 (ptr, ...) @printf(ptr @result_format, i32 %wide)\n",
                    result.llvm(),
                    result.llvm()
                ),
            )
        }
        Ty::Float32 | Ty::Float64 => {
            let (bits, format) = if result == Ty::Float32 {
                (32, "(float32 (bits %08x))\n")
            } else {
                (64, "(float64 (bits %016llx))\n")
            };
            (
                format,
                format!(
                    "  %value = call {} @flurry_{root}()\n  %bits = bitcast {} %value to i{bits}\n  %output_write = call i32 (ptr, ...) @printf(ptr @result_format, i{bits} %bits)\n",
                    result.llvm(),
                    result.llvm()
                ),
            )
        }
        Ty::Int(32, false) if **logical_result == Term::Primitive(P::Char) => (
            "(char %u)\n",
            format!(
                "  %value = call i32 @flurry_{root}()\n  %output_write = call i32 (ptr, ...) @printf(ptr @result_format, i32 %value)\n"
            ),
        ),
        Ty::Int(0, false) => (
            "(unsigned-integer 0)\n",
            format!(
                "  call {{}} @flurry_{root}()\n  %output_write = call i32 (ptr, ...) @printf(ptr @result_format)\n"
            ),
        ),
        Ty::Int(128, signed) => {
            module.push_str(include_str!("print_i128.ll"));
            (
                if signed {
                    "(signed-integer %s)\n"
                } else {
                    "(unsigned-integer %s)\n"
                },
                format!(
                    "  %buffer = alloca [41 x i8]\n  %value = call i128 @flurry_{root}()\n  %text = call ptr @flurry_decimal_i128(i128 %value, i1 {signed}, ptr %buffer)\n  %output_write = call i32 (ptr, ...) @printf(ptr @result_format, ptr %text)\n"
                ),
            )
        }
        Ty::Int(bits, signed) if bits > 64 && bits <= bits::MAX_BIT_WIDTH => {
            module.push_str(include_str!("print_big_integer.ll"));
            module.push_str(&decimal::definition(bits, &mut fuel)?);
            (
                if signed {
                    "(signed-integer %s)\n"
                } else {
                    "(unsigned-integer %s)\n"
                },
                format!(
                    "  %buffer = alloca [{} x i8]\n  %value = call i{bits} @flurry_{root}()\n  %text = call ptr @flurry_decimal_i{bits}(i{bits} %value, i1 {signed}, ptr %buffer)\n  %output_write = call i32 (ptr, ...) @printf(ptr @result_format, ptr %text)\n",
                    decimal::buffer_size(bits)
                ),
            )
        }
        Ty::Int(bits, signed) if bits <= 64 => {
            let extension = if bits < 64 {
                format!(
                    "  %wide = {} i{bits} %value to i64\n",
                    if signed { "sext" } else { "zext" }
                )
            } else {
                String::new()
            };
            (
                if signed {
                    "(signed-integer %lld)\n"
                } else {
                    "(unsigned-integer %llu)\n"
                },
                format!(
                    "  %value = call i{bits} @flurry_{root}()\n{extension}  %output_write = call i32 (ptr, ...) @printf(ptr @result_format, i64 %{})\n",
                    if bits < 64 { "wide" } else { "value" }
                ),
            )
        }
        _ => {
            return Err("host entry result printer does not support this representation".into());
        }
    };
    let escaped = format
        .bytes()
        .map(|b| {
            if b == b'\n' {
                "\\0A".into()
            } else {
                (b as char).to_string()
            }
        })
        .collect::<String>();
    module.push_str(&format!(
        "@result_format = private constant [{} x i8] c\"{escaped}\\00\"\n\n",
        format.len() + 1
    ));
    module.push_str(&functions);
    let body = if checked {
        body.replace(&format!("@flurry_{root}()"), "@flurry_entry()")
    } else {
        body
    };
    module.push_str(&format!(
        "define i32 @main() {{\nentry:\n{body}  call void @flurry_output_record(i32 %output_write)\n  %status = call i32 @flurry_output_finish()\n  ret i32 %status\n}}\n"
    ));
    Ok(module)
}

fn literal_value(literal: &Literal, ty: Ty) -> Result<Value, String> {
    let text = match literal {
        Literal::Float16(bits) => format!("bitcast (i16 {bits} to half)"),
        Literal::BFloat16(bits) => format!("bitcast (i16 {bits} to bfloat)"),
        Literal::Float32(bits) => format!("bitcast (i32 {bits} to float)"),
        Literal::Float64(bits) => format!("bitcast (i64 {bits} to double)"),
        Literal::BitVec { width: 0, .. } => "zeroinitializer".into(),
        Literal::BitVec { width, words } if *width <= bits::MAX_BIT_WIDTH as usize => {
            bits::decimal_words(words)
        }
        Literal::Bool(b) => b.to_string(),
        Literal::SignedInteger(n) => n.to_string(),
        Literal::UnsignedInteger(n) => n.to_string(),
        Literal::Char(c) => (*c as u32).to_string(),
        _ => return Err("host literal is not implemented".into()),
    };

    Ok(Value { ty, text })
}

/// Admit only cleanup programs whose Invoke responsibilities are lowered at
/// supported sites. Release-only storage may live until the LLVM function ends;
/// ownership verification has already ruled out use after transfer/release.
pub(super) fn validate_cleanup(
    graph: &iir::Package,
    schedule: &iir::cleanup::Schedule,
    fuel: &mut usize,
) -> Result<(), String> {
    use iir::cleanup::Step;
    for alias in &schedule.aliases {
        cleanup::validate_alias(schedule, alias, fuel)?;
        if let Some(iir::cleanup::ContextKey::Function { callee, .. }) = alias.context.last() {
            *fuel = fuel
                .checked_sub(graph.functions.len())
                .ok_or("host cleanup alias graph budget exhausted")?;
            if graph.function(callee).is_none() {
                return Err("host cleanup alias callee is absent from the graph".into());
            }
        }
    }
    let mut representations = Vec::new();
    for function in &schedule.functions {
        for (program, source, boundary) in function
            .sites
            .iter()
            .flat_map(|site| {
                site.actions
                    .iter()
                    .map(move |action| (&action.program, site.source, Some(site.boundary)))
            })
            .chain(
                function
                    .discards
                    .iter()
                    .map(|discard| (&discard.program, discard.source, None)),
            )
        {
            let direct = cleanup::has_invoke(&program.steps)
                && cleanup::admitted_direct_program(graph, schedule, function, program, fuel);
            let mut pending: Vec<_> = program.steps.iter().collect();
            while let Some(step) = pending.pop() {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or("host cleanup admission budget exhausted")?;
                match step {
                    Step::Release { ty, .. } => {
                        // An error-only result retains its impossible success
                        // alternative in the logical cleanup tree. Releasing
                        // that Never leaf is bookkeeping, not materializing a
                        // Never value. Actual program/owner storage still goes
                        // through repr below, and Invoke is checked separately.
                        if *ty != Term::Primitive(P::Never) {
                            representations.push(ty);
                        }
                    }
                    Step::IfConstructed { steps, .. } => pending.extend(steps),
                    Step::Switch { branches, .. } => pending.extend(branches.iter().flatten()),
                    Step::Select { cases } => {
                        pending.extend(cases.iter().flat_map(|case| &case.steps))
                    }
                    Step::Invoke { method, .. } if !direct => {
                        // Preserve the actual closure scheduling failure instead
                        // of disguising it as missing destructor lowering.
                        if let Some(index) = function.context.iter().rposition(|context| {
                            matches!(context, iir::cleanup::ContextKey::Closure { .. })
                        }) {
                            let node = closures::definition(
                                graph,
                                schedule,
                                function,
                                &function.context[..=index],
                                fuel,
                            )?;
                            closures::schedules_for(graph, schedule, node, fuel)?;
                        }
                        return Err(format!(
                            "host destructor invocation is not implemented: {method}; cleanup cannot be discarded; at {}..{} ({}) in {}",
                            source.start,
                            source.end,
                            boundary
                                .map_or_else(|| "discard".into(), |boundary| boundary.to_string()),
                            function.instance
                        ));
                    }
                    Step::Invoke { .. } => {}
                }
            }
            representations.push(&program.ty);
        }
    }
    for ty in representations {
        repr(graph, ty, true)?;
    }
    Ok(())
}

#[cfg(test)]
mod output_status_tests {
    use crate::host::{OptimizationLevel, Toolchain};

    #[test]
    fn native_output_status_retains_early_errors_after_later_success() {
        for optimization in [OptimizationLevel::O0, OptimizationLevel::O3] {
            for (character, prefix, flush, expected) in [
                (65, 3, 0, 0),
                (-1, 3, 0, 1),
                (65, -1, 0, 1),
                (65, 3, -1, 1),
                (-1, -1, -1, 1),
            ] {
                // Inject independent stdio outcomes, including a successful
                // later write and flush after an immediate failure.
                let runtime = include_str!("output.ll")
                    .replace(
                        "declare i32 @putchar(i32)",
                        "define i32 @putchar(i32 %value) { ret i32 %value }",
                    )
                    .replace(
                        "declare i32 @fflush(ptr)",
                        &format!("define i32 @fflush(ptr %stream) {{ ret i32 {flush} }}"),
                    );
                let ir = format!(
                    r#"
                    define i32 @printf(ptr %format, ...) {{ ret i32 {prefix} }}
                    {runtime}
                    define i32 @main() {{
                      call i32 @flurry_output_char(i32 {character})
                      call void @flurry_output_prefix(ptr null)
                      call i32 @flurry_output_char(i32 66)
                      %status = call i32 @flurry_output_finish()
                      %correct = icmp eq i32 %status, {expected}
                      %exit = select i1 %correct, i32 0, i32 1
                      ret i32 %exit
                    }}
                "#
                );
                let output = Toolchain {
                    optimization,
                    ..Toolchain::default()
                }
                .execute_test_ir(&ir)
                .unwrap();
                assert!(
                    output.status.success(),
                    "{character}/{prefix}/{flush}: {output:?}"
                );
                assert!(output.stdout.is_empty());
                assert!(output.stderr.is_empty());
            }
        }
    }
}

#[cfg(test)]
mod recursive_layout_tests {
    use super::*;

    #[test]
    fn pointer_edges_break_layout_recursion_but_value_edges_do_not() {
        let declaration = nir::DeclarationId {
            package: 0,
            local: 1,
        };
        let node = Term::Nominal {
            declaration,
            arguments: vec![],
        };
        let make_graph = |field| iir::Package {
            nominals: vec![iir::NominalInstance {
                ty: node.clone(),
                definition: iir::NominalDefinition::Struct(nir::program::Struct {
                    declaration,
                    name: "Node".into(),
                    parameter_types: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    type_parameters: vec![],
                    fields: vec![("next".into(), field)],
                }),
            }],
            ..Default::default()
        };
        let pointer = Term::Pointer(Box::new(node.clone()));
        let graph = make_graph(pointer.clone());
        assert_eq!(repr(&graph, &node, true).unwrap().llvm(), "{ ptr }");
        assert_eq!(repr(&graph, &pointer, true).unwrap().llvm(), "ptr");
        assert!(
            repr(&graph, &pointer, false)
                .unwrap_err()
                .contains("resource verification")
        );
        let graph = make_graph(node.clone());
        assert!(
            repr(&graph, &node, true)
                .unwrap_err()
                .contains("nesting budget")
        );
    }
}

#[cfg(test)]
mod layout_budget_tests {
    use super::*;

    fn doubling_graph(depth: u32) -> (iir::Package, Term) {
        let mut graph = iir::Package::default();
        let mut ty = Term::Primitive(P::Bool);
        for local in 0..depth {
            let declaration = nir::DeclarationId { package: 0, local };
            let next = Term::Nominal {
                declaration,
                arguments: vec![],
            };
            graph.nominals.push(iir::NominalInstance {
                ty: next.clone(),
                definition: iir::NominalDefinition::Struct(nir::program::Struct {
                    declaration,
                    name: format!("Level{local}"),
                    parameter_types: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    type_parameters: vec![],
                    fields: vec![("left".into(), ty.clone()), ("right".into(), ty)],
                }),
            });
            ty = next;
        }
        (graph, ty)
    }

    #[test]
    fn shallow_nominal_dag_cannot_expand_without_a_node_limit() {
        let (graph, ty) = doubling_graph(20);
        assert_eq!(graph.nominals.len(), 20);
        assert!(repr(&graph, &ty, true).unwrap_err().contains("node budget"));
        // Opaque pointer representation must not expand the large pointee.
        assert_eq!(
            repr(&graph, &Term::Pointer(Box::new(ty)), true).unwrap(),
            Ty::Pointer
        );
        let (graph, ty) = doubling_graph(8);
        assert!(
            repr_at(&graph, &ty, 0, true, &mut 510)
                .unwrap_err()
                .contains("node budget")
        );
        let mut remaining = 511;
        let layout = repr_at(&graph, &ty, 0, true, &mut remaining).unwrap();
        assert_eq!(remaining, 0);
        assert_eq!(layout.llvm().matches("i1").count(), 256);
    }

    #[test]
    fn sibling_layouts_share_budget_in_products_closures_and_sums() {
        let graph = iir::Package::default();
        let boolean = Term::Primitive(P::Bool);
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(boolean.clone()),
        };
        for (ty, cost) in [
            (Term::Tuple(vec![boolean.clone(), boolean.clone()]), 3),
            (
                Term::Record(vec![
                    ("a".into(), boolean.clone()),
                    ("b".into(), boolean.clone()),
                ]),
                3,
            ),
            (
                Term::Closure {
                    environment: vec![boolean.clone(), boolean.clone()],
                    signature: Box::new(signature),
                },
                4,
            ),
            (
                Term::ErrorResult {
                    errors: vec![boolean.clone()],
                    result: Box::new(boolean.clone()),
                },
                3,
            ),
        ] {
            assert!(
                repr_at(&graph, &ty, 0, true, &mut { cost - 1 })
                    .unwrap_err()
                    .contains("node budget")
            );
            let mut remaining = cost;
            repr_at(&graph, &ty, 0, true, &mut remaining).unwrap();
            assert_eq!(remaining, 0);
        }
        let definition = nir::program::Enum {
            is_intrinsic: false,
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration: nir::DeclarationId {
                package: 0,
                local: 0,
            },
            name: "Choice".into(),
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
            variants: vec![
                nir::program::Variant {
                    name: "none".into(),
                    shape: nir::program::VariantShape::Unit,
                    fields: vec![],
                },
                nir::program::Variant {
                    name: "some".into(),
                    shape: nir::program::VariantShape::Tuple,
                    fields: vec![boolean],
                },
            ],
        };
        assert!(
            enums::EnumLayout::build(&graph, &definition, 0, true, &mut 2)
                .unwrap_err()
                .contains("node budget")
        );
        let mut remaining = 3;
        enums::EnumLayout::build(&graph, &definition, 0, true, &mut remaining).unwrap();
        assert_eq!(remaining, 0);
    }
}
