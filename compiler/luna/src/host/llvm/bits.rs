use super::*;
// Host compilation resource limit, not a source-language width restriction.
pub(super) const MAX_BIT_WIDTH: u32 = 65_536;

pub(super) fn decimal_words(words: &[u32]) -> String {
    let mut digits = vec![0u64];
    for word in words.iter().rev() {
        let mut carry = u64::from(*word);
        for digit in &mut digits {
            let value = (*digit << 32) + carry;
            *digit = value % 1_000_000_000;
            carry = value / 1_000_000_000;
        }
        while carry != 0 {
            digits.push(carry % 1_000_000_000);
            carry /= 1_000_000_000;
        }
    }
    let mut output = digits.pop().unwrap().to_string();
    for digit in digits.iter().rev() {
        output.push_str(&format!("{digit:09}"));
    }
    output
}

pub(super) fn bit_width(ty: &Ty) -> Option<u32> {
    match ty {
        Ty::Int(width, _) => Some(*width),
        Ty::Float16 | Ty::BFloat16 => Some(16),
        Ty::Float32 => Some(32),
        Ty::Float64 => Some(64),
        Ty::Enum(layout) if !layout.aggregate && layout.symbols.is_none() => Some(layout.width),
        _ => None,
    }
}
impl Emitter<'_> {
    pub(super) fn to_bits(&mut self, value: Value) -> Result<Value, String> {
        let width = bit_width(&value.ty)
            .ok_or("host bit view is not implemented for this representation")?;
        if matches!(
            value.ty,
            Ty::Float16 | Ty::BFloat16 | Ty::Float32 | Ty::Float64
        ) {
            Ok(self.instruction(
                Ty::Int(width, false),
                format!("bitcast {} {} to i{width}", value.ty.llvm(), value.text),
            ))
        } else {
            Ok(Value {
                ty: Ty::Int(width, false),
                text: value.text,
            })
        }
    }
    pub(super) fn from_bits(&mut self, value: Value, target: Ty) -> Result<Value, String> {
        let width = bit_width(&value.ty).ok_or("host bit conversion source has no width")?;
        let target_width = bit_width(&target).ok_or("host bit conversion target has no width")?;
        if width != target_width {
            return Err("host reinterpretation changes width".into());
        }
        if matches!(
            target,
            Ty::Float16 | Ty::BFloat16 | Ty::Float32 | Ty::Float64
        ) {
            Ok(self.instruction(
                target.clone(),
                format!("bitcast i{width} {} to {}", value.text, target.llvm()),
            ))
        } else {
            Ok(Value {
                ty: target,
                text: value.text,
            })
        }
    }
    pub(super) fn bit_convert(&mut self, value: Value, target: Ty) -> Result<Value, String> {
        let value = self.to_bits(value)?;
        let source = bit_width(&value.ty).unwrap();
        let width = bit_width(&target).ok_or("host bit conversion target unsupported")?;
        if source < width && matches!(target, Ty::Int(_, false)) {
            if source == 0 {
                Ok(Value {
                    ty: target,
                    text: "0".into(),
                })
            } else {
                Ok(self.instruction(target, format!("zext i{source} {} to i{width}", value.text)))
            }
        } else {
            self.from_bits(value, target)
        }
    }
}

impl Emitter<'_> {
    pub(super) fn bit_binary(&mut self, op: &str, a: Value, b: Value) -> Result<Value, String> {
        if a.ty != b.ty || !matches!(a.ty, Ty::Int(_, false)) {
            return Err("host bit operands must have the same width".into());
        }
        if a.ty == Ty::Int(0, false) {
            return Ok(if op == "icmp eq" {
                Value {
                    ty: Ty::Bool,
                    text: "true".into(),
                }
            } else {
                a
            });
        }
        let ty = if op == "icmp eq" {
            Ty::Bool
        } else {
            a.ty.clone()
        };
        Ok(self.instruction(ty, format!("{op} {} {}, {}", a.ty.llvm(), a.text, b.text)))
    }
    pub(super) fn bit_concat(&mut self, a: Value, b: Value) -> Result<Value, String> {
        let m = bit_width(&a.ty).ok_or("host bit concat input has no width")?;
        let n = bit_width(&b.ty).ok_or("host bit concat input has no width")?;
        if m == 0 {
            return Ok(b);
        }
        if n == 0 {
            return Ok(a);
        }
        let width = m + n;
        if width > MAX_BIT_WIDTH {
            return Err("host bit concat exceeds the host width budget".into());
        }
        let ty = Ty::Int(width, false);
        let a = self.instruction(ty.clone(), format!("zext i{m} {} to i{width}", a.text));
        let b = self.instruction(ty.clone(), format!("zext i{n} {} to i{width}", b.text));
        let a = self.instruction(ty.clone(), format!("shl i{width} {}, {n}", a.text));
        self.bit_binary("or", a, b)
    }
    pub(super) fn bit_slice(
        &mut self,
        value: Value,
        start: usize,
        end: usize,
    ) -> Result<Value, String> {
        let width = bit_width(&value.ty).ok_or("host bit slice input has no width")?;
        if start > end || end > width as usize {
            return Err("host bit slice requires a in-range selection".into());
        }
        if start == end {
            return Ok(empty_bits());
        }
        let value = if start == 0 {
            value
        } else {
            self.instruction(
                value.ty.clone(),
                format!("lshr i{width} {}, {start}", value.text),
            )
        };
        let size = (end - start) as u32;
        if size == width {
            Ok(value)
        } else {
            Ok(self.instruction(
                Ty::Int(size, false),
                format!("trunc i{width} {} to i{size}", value.text),
            ))
        }
    }
    fn bit_position(&mut self, value: Value) -> Result<Value, String> {
        let Ty::Int(width, signed) = value.ty else {
            return Err("host bit position needs a machine integer".into());
        };
        if width == 128 {
            return Ok(value);
        }
        let op = if signed { "sext" } else { "zext" };
        Ok(self.instruction(
            Ty::Int(128, false),
            format!("{op} i{width} {} to i128", value.text),
        ))
    }
    pub(super) fn bit_window(
        &mut self,
        value: Value,
        start: Value,
        end: Option<Value>,
        size: u32,
    ) -> Result<Value, String> {
        let width = bit_width(&value.ty).ok_or("host bit window input has no width")?;
        if size > width {
            return Err("host bit window needs a in-range width".into());
        }
        let start = self.bit_position(start)?;
        // Check before narrowing or shifting: LLVM shifts by the input width
        // are poison, including positions with high bits lost by truncation.
        let valid = self.instruction(
            Ty::Bool,
            format!("icmp ule i128 {}, {}", start.text, width - size),
        );
        self.guard(&valid)?;
        if let Some(end) = end {
            let end = self.bit_position(end)?;
            let expected = self.instruction(
                Ty::Int(128, false),
                format!("add i128 {}, {size}", start.text),
            );
            let valid = self.instruction(
                Ty::Bool,
                format!("icmp eq i128 {}, {}", end.text, expected.text),
            );
            self.guard(&valid)?;
        }
        if size == 0 {
            return Ok(empty_bits());
        }
        let shift = if width == 128 {
            start
        } else {
            self.instruction(
                Ty::Int(width, false),
                format!(
                    "{} i128 {} to i{width}",
                    if width < 128 { "trunc" } else { "zext" },
                    start.text
                ),
            )
        };
        let value = self.instruction(
            Ty::Int(width, false),
            format!("lshr i{width} {}, {}", value.text, shift.text),
        );
        self.bit_slice(value, 0, size as usize)
    }
}

impl Emitter<'_> {
    pub(super) fn bit_write(
        &mut self,
        slot: Slot,
        selections: &[nir::program::BitSelection],
        rhs: &Expr,
        depth: usize,
    ) -> Result<Value, String> {
        use nir::program::BitSelection as S;
        let width =
            bit_width(&slot.ty).ok_or("host bit write storage has no bit representation")?;
        let mut size = width;
        let mut offset = Value {
            ty: Ty::Int(128, false),
            text: "0".into(),
        };
        for selection in selections {
            let (start, end, selected) = match selection {
                S::Static { start, end } => {
                    if start > end || *end > size as usize {
                        return Err("host bit write requires a valid selection".into());
                    }
                    (
                        Value {
                            ty: Ty::Int(128, false),
                            text: start.to_string(),
                        },
                        None,
                        (end - start) as u32,
                    )
                }
                S::Index(index) => {
                    let index = self.expr(index, depth)?;
                    if self.terminated {
                        return Ok(Value::unit());
                    }
                    (self.bit_position(index)?, None, 1)
                }
                S::Window { start, end, width } => {
                    let start = self.expr(start, depth)?;
                    let end = self.expr(end, depth)?;
                    if self.terminated {
                        return Ok(Value::unit());
                    }
                    (
                        self.bit_position(start)?,
                        Some(self.bit_position(end)?),
                        u32::try_from(*width).map_err(|_| "host bit selection width too large")?,
                    )
                }
            };
            if selected > size {
                return Err("host bit write selection width unsupported".into());
            }
            let valid = self.instruction(
                Ty::Bool,
                format!("icmp ule i128 {}, {}", start.text, size - selected),
            );
            self.guard(&valid)?;
            if let Some(end) = end {
                let expected = self.instruction(
                    Ty::Int(128, false),
                    format!("add i128 {}, {selected}", start.text),
                );
                let valid = self.instruction(
                    Ty::Bool,
                    format!("icmp eq i128 {}, {}", end.text, expected.text),
                );
                self.guard(&valid)?;
            }
            offset = self.instruction(
                Ty::Int(128, false),
                format!("add i128 {}, {}", offset.text, start.text),
            );
            size = selected;
        }
        // Positions are now frozen. Read preserved bits only after evaluating
        // the RHS, which may write the same storage or its aggregate parent.
        let rhs = self.expr(rhs, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        if rhs.ty != Ty::Int(size, false) {
            return Err("host bit write RHS width mismatch".into());
        }
        if size == 0 {
            return Ok(Value::unit());
        }
        let old = self.load(&slot);
        let old = self.to_bits(old)?;
        let ty = Ty::Int(width, false);
        let shift = if width == 128 {
            offset
        } else {
            self.instruction(
                ty.clone(),
                format!(
                    "{} i128 {} to i{width}",
                    if width < 128 { "trunc" } else { "zext" },
                    offset.text
                ),
            )
        };
        let rhs = if size == width {
            rhs
        } else {
            self.instruction(ty.clone(), format!("zext i{size} {} to i{width}", rhs.text))
        };
        let rhs = self.instruction(
            ty.clone(),
            format!("shl i{width} {}, {}", rhs.text, shift.text),
        );
        let mask = if size == width {
            Value {
                ty: ty.clone(),
                text: "-1".into(),
            }
        } else {
            self.instruction(ty.clone(), format!("lshr i{width} -1, {}", width - size))
        };
        let mask = self.instruction(
            ty.clone(),
            format!("shl i{width} {}, {}", mask.text, shift.text),
        );
        let inverse = self.instruction(ty.clone(), format!("xor i{width} {}, -1", mask.text));
        let kept = self.bit_binary("and", old, inverse)?;
        let result = self.bit_binary("or", kept, rhs)?;
        let result = self.from_bits(result, slot.ty.clone())?;
        self.store(&slot, &result)?;
        Ok(Value::unit())
    }
}

fn empty_bits() -> Value {
    Value {
        ty: Ty::Int(0, false),
        text: "zeroinitializer".into(),
    }
}
