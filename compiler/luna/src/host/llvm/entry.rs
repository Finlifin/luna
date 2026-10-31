//! Development entry printer for structural products. This is not a language ABI.
use super::*;

pub(super) fn product_main(
    graph: &iir::Package,
    logical: &Term,
    physical: &Ty,
    target: &str,
    checked: bool,
    fuel: &mut usize,
) -> Result<String, String> {
    let mut printer = Printer {
        graph,
        checked,
        fuel,
        serial: 0,
        body: String::new(),
        args: Vec::new(),
        globals: String::new(),
        strings: false,
        wide: false,
        widths: BTreeSet::new(),
    };
    printer.value(logical, physical, "%value", 0)?;
    printer.print("\n")?;
    let mut module = String::from(
        "@bool_true = private constant [5 x i8] c\"true\\00\"\n@bool_false = private constant [6 x i8] c\"false\\00\"\n",
    );
    if printer.wide {
        module.push_str(include_str!("../print_i128.ll"));
    }
    if !printer.widths.is_empty() {
        module.push_str(include_str!("../print_big_integer.ll"));
        for width in &printer.widths {
            module.push_str(&decimal::definition(*width, printer.fuel)?);
        }
    }
    if printer.strings {
        module.push_str(include_str!("../print_string.ll"));
    }
    module.push_str(&printer.globals);
    let call = if *physical == Ty::Unit {
        format!("call void @{target}()")
    } else {
        format!("%value = call {} @{target}()", physical.llvm())
    };
    module.push_str(&format!(
        "define i32 @main() {{\nentry:\n  {call}\n{}  %status = call i32 @flurry_output_finish()\n  ret i32 %status\n}}\n",
        printer.body
    ));
    Ok(module)
}
struct Printer<'a> {
    graph: &'a iir::Package,
    checked: bool,
    fuel: &'a mut usize,
    serial: usize,
    body: String,
    args: Vec<String>,
    globals: String,
    strings: bool,
    wide: bool,
    widths: BTreeSet<u32>,
}
impl Printer<'_> {
    fn print(&mut self, format: &str) -> Result<(), String> {
        use std::fmt::Write;
        *self.fuel = self
            .fuel
            .checked_sub(format.len().saturating_add(1))
            .ok_or("host entry format budget exhausted")?;
        let name = format!("flurry_display_format{}", self.serial);
        self.serial += 1;
        let mut escaped = String::new();
        for byte in format.bytes() {
            write!(&mut escaped, "\\{byte:02X}").expect("writing to a String cannot fail");
        }
        self.globals.push_str(&format!(
            "@{name} = private constant [{} x i8] c\"{escaped}\\00\"\n",
            format.len() + 1
        ));
        let args = std::mem::take(&mut self.args)
            .into_iter()
            .map(|a| format!(", {a}"))
            .collect::<String>();
        let result = self.instruction(format!("call i32 (ptr, ...) @printf(ptr @{name}{args})"));
        self.body.push_str(&format!(
            "  call void @flurry_output_record(i32 {result})\n"
        ));
        Ok(())
    }
    fn instruction(&mut self, code: String) -> String {
        let name = format!("%display{}", self.serial);
        self.serial += 1;
        self.body.push_str(&format!("  {name} = {code}\n"));
        name
    }
    fn value(
        &mut self,
        logical: &Term,
        physical: &Ty,
        value: &str,
        depth: usize,
    ) -> Result<(), String> {
        if depth >= 128 || *self.fuel == 0 {
            return Err("host entry display budget exhausted".into());
        }
        *self.fuel -= 1;
        if let Term::ErrorResult { errors, result } = logical {
            nir::type_computations::charge_term(logical, self.fuel)
                .map_err(|_| "host entry error type budget exhausted")?;
            let Ty::ErrorResult(layout) = physical else {
                return Err("host entry error result layout mismatch".into());
            };
            if repr(self.graph, logical, self.checked)? != *physical {
                return Err("host entry error result layout mismatch".into());
            }
            *self.fuel = self
                .fuel
                .checked_sub(layout.len())
                .ok_or("host entry error row budget exhausted")?;
            self.print(&format!("(error-result (type {logical}) ").replace('%', "%%"))?;
            let tag = self.instruction(format!("extractvalue {} {value}, 0", physical.llvm()));
            let base = self.serial;
            self.serial += 1;
            let end = format!("display_error_end{base}");
            let bad = format!("display_error_bad{base}");
            let branches = std::iter::once(result.as_ref())
                .chain(errors.iter())
                .zip(layout.iter())
                .enumerate()
                .filter(|(index, (ty, _))| *index != 0 || **ty != Term::Primitive(P::Never))
                .collect::<Vec<_>>();
            self.body
                .push_str(&format!("  switch i32 {tag}, label %{bad} ["));
            for (index, _) in &branches {
                self.body.push_str(&format!(
                    " i32 {index}, label %display_error_{base}_{index}"
                ));
            }
            self.body.push_str(" ]\n");
            for (index, (ty, (_, field))) in branches {
                self.body
                    .push_str(&format!("display_error_{base}_{index}:\n"));
                if index == 0 {
                    self.print("(branch success) (value ")?;
                } else {
                    self.print(&format!("(branch (error {})) (value ", index - 1))?;
                }
                let payload = if *field == Ty::Unit {
                    String::new()
                } else {
                    self.instruction(format!(
                        "extractvalue {} {value}, {}",
                        physical.llvm(),
                        index + 1
                    ))
                };
                self.value(ty, field, &payload, depth + 1)?;
                self.print("))")?;
                self.body.push_str(&format!("  br label %{end}\n"));
            }
            self.body.push_str(&format!(
                "{bad}:\n  call void @llvm.trap()\n  unreachable\n{end}:\n"
            ));
            return Ok(());
        }
        // Admit logical structural products only. Nominal ownership cannot be
        // discarded merely because its physical layout looks like a record.
        let fields = match logical {
            Term::Record(fields) => Some((
                "anonymous-record (fields",
                fields
                    .iter()
                    .map(|(name, ty)| (Some(name.as_str()), ty))
                    .collect::<Vec<_>>(),
            )),
            Term::Tuple(fields) => Some(("tuple", fields.iter().map(|ty| (None, ty)).collect())),
            Term::NormalForm { .. } => self
                .graph
                .normal_forms
                .iter()
                .find_map(|head| head.tuple_elements(logical))
                .map(|fields| ("tuple", fields.iter().map(|ty| (None, ty)).collect())),
            _ => None,
        };
        if let Some((tag, fields)) = fields {
            // Match the interpreter's logical value display, including the
            // explicit fields container for anonymous records.
            let close = if matches!(logical, Term::Record(_)) {
                "))"
            } else {
                ")"
            };
            if fields.is_empty() && *physical == Ty::Unit {
                self.print(&format!("({tag}{close}"))?;
                return Ok(());
            }
            let Ty::Record(layout) = physical else {
                return Err("host entry product layout mismatch".into());
            };
            if fields.len() != layout.len() {
                return Err("host entry product field count mismatch".into());
            }
            self.print(&format!("({tag}"))?;
            for (index, ((name, ty), field)) in fields.into_iter().zip(layout.iter()).enumerate() {
                let child = if *field == Ty::Unit {
                    String::new()
                } else {
                    self.instruction(format!("extractvalue {} {value}, {index}", physical.llvm()))
                };
                if let Some(name) = name {
                    *self.fuel = self
                        .fuel
                        .checked_sub(name.len().saturating_add(1))
                        .ok_or("host entry field name budget exhausted")?;
                    let name = Literal::String(name.to_owned())
                        .to_string()
                        .replace('%', "%%");
                    self.print(&format!(" (field {name} "))?;
                } else {
                    self.print(" ")?;
                }
                self.value(ty, field, &child, depth + 1)?;
                if name.is_some() {
                    self.print(")")?;
                }
            }
            self.print(close)?;
            return Ok(());
        }
        if !matches!(logical, Term::Primitive(_))
            || repr(self.graph, logical, self.checked)? != *physical
        {
            return Err("host entry product printer requires structural products with primitive leaves; owned nominal values need an entry cleanup adapter".into());
        }
        if matches!(physical, Ty::String | Ty::Symbol) {
            let printer = if *physical == Ty::Symbol {
                "flurry_print_symbol"
            } else {
                "flurry_print_string"
            };
            self.strings = true;
            self.body
                .push_str(&format!("  call void @{printer}({{ ptr, i64 }} {value})\n"));
            return Ok(());
        }
        let format: String = match physical {
            Ty::Unit => "(unit)".into(),
            Ty::Bool => {
                let text = self.instruction(format!(
                    "select i1 {value}, ptr @bool_true, ptr @bool_false"
                ));
                self.args.push(format!("ptr {text}"));
                "(bool %s)".into()
            }
            Ty::Int(32, false) if *logical == Term::Primitive(P::Char) => {
                self.args.push(format!("i32 {value}"));
                "(char %u)".into()
            }
            Ty::Int(0, false) => "(unsigned-integer 0)".into(),
            Ty::Int(bits, signed) if *bits <= 64 => {
                let wide = if *bits < 64 {
                    self.instruction(format!(
                        "{} i{bits} {value} to i64",
                        if *signed { "sext" } else { "zext" }
                    ))
                } else {
                    value.to_owned()
                };
                self.args.push(format!("i64 {wide}"));
                if *signed {
                    "(signed-integer %lld)"
                } else {
                    "(unsigned-integer %llu)"
                }
                .into()
            }
            Ty::Int(128, signed) => {
                self.wide = true;
                let buffer = self.instruction("alloca [41 x i8]".into());
                let text = self.instruction(format!(
                    "call ptr @flurry_decimal_i128(i128 {value}, i1 {signed}, ptr {buffer})"
                ));
                self.args.push(format!("ptr {text}"));
                if *signed {
                    "(signed-integer %s)"
                } else {
                    "(unsigned-integer %s)"
                }
                .into()
            }
            Ty::Int(bits, signed) if *bits > 64 && *bits <= bits::MAX_BIT_WIDTH => {
                self.widths.insert(*bits);
                let buffer =
                    self.instruction(format!("alloca [{} x i8]", decimal::buffer_size(*bits)));
                let text = self.instruction(format!(
                    "call ptr @flurry_decimal_i{bits}(i{bits} {value}, i1 {signed}, ptr {buffer})"
                ));
                self.args.push(format!("ptr {text}"));
                if *signed {
                    "(signed-integer %s)"
                } else {
                    "(unsigned-integer %s)"
                }
                .into()
            }
            Ty::Float16 | Ty::BFloat16 => {
                let bits = if *physical == Ty::BFloat16 {
                    value.to_string()
                } else {
                    self.instruction(format!("bitcast {} {value} to i16", physical.llvm()))
                };
                let wide = self.instruction(format!("zext i16 {bits} to i32"));
                self.args.push(format!("i32 {wide}"));
                if *physical == Ty::Float16 {
                    "(float16 (bits %04x))"
                } else {
                    "(bfloat16 (bits %04x))"
                }
                .into()
            }
            Ty::Float32 | Ty::Float64 => {
                let width = if *physical == Ty::Float32 { 32 } else { 64 };
                let bits =
                    self.instruction(format!("bitcast {} {value} to i{width}", physical.llvm()));
                self.args.push(format!("i{width} {bits}"));
                if width == 32 {
                    "(float32 (bits %08x))"
                } else {
                    "(float64 (bits %016llx))"
                }
                .into()
            }
            _ => return Err("host entry product has an unsupported primitive leaf".into()),
        };
        self.print(&format)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_product_names_require_budget_and_recover() {
        let graph = iir::Package::default();
        let name = "field".repeat(1024);
        let logical = Term::Record(vec![(name.clone(), Term::Primitive(P::Bool))]);
        let physical = Ty::Record(std::sync::Arc::new(vec![Ty::Bool]));
        let error = product_main(&graph, &logical, &physical, "run", false, &mut 1000).unwrap_err();
        assert_eq!(error, "host entry field name budget exhausted");
        let output = product_main(&graph, &logical, &physical, "run", false, &mut 100000).unwrap();
        assert_eq!(
            product_main(&graph, &logical, &physical, "run", false, &mut 100000).unwrap(),
            output
        );
        assert!(output.contains("extractvalue { i1 } %value, 0"));
    }
    #[test]
    fn escaped_product_names_are_data_not_printf_directives() {
        let graph = iir::Package::default();
        let logical = Term::Record(vec![("%s\n\"中".into(), Term::Primitive(P::Bool))]);
        let physical = Ty::Record(std::sync::Arc::new(vec![Ty::Bool]));
        let output = product_main(&graph, &logical, &physical, "run", false, &mut 10000).unwrap();
        assert!(output.contains("\\25\\25\\73"));
        assert!(output.contains("\\5C\\6E"));
        assert!(output.contains("\\E4\\B8\\AD"));
    }
}
