use super::*;

impl Emitter<'_> {
    pub(super) fn text_query(
        &mut self,
        expr: &Expr,
        subject: &Expr,
        query: nir::program::TextQueryKind,
        depth: usize,
    ) -> Result<Value, String> {
        if query.is_comptime_only() {
            return Err("comptime text operation is unavailable at runtime".into());
        }
        if !query.validate(&subject.ty, &expr.ty) {
            return Err("host text query domain mismatch".into());
        }
        let packet = self.expr(subject, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let text = self.record_field(&packet, 0)?;
        let expected = if query.is_symbol() {
            Ty::Symbol
        } else {
            Ty::String
        };
        if text.ty != expected {
            return Err("host byte query subject representation mismatch".into());
        }
        let length = self.instruction(
            Ty::Int(64, false),
            format!("extractvalue {} {}, 1", text.ty.llvm(), text.text),
        );
        if query.is_length() {
            return Ok(length);
        }
        if query == nir::program::TextQueryKind::Slice {
            let start = self.record_field(&packet, 1)?;
            let end = self.record_field(&packet, 2)?;
            let order_ok = self.instruction(
                Ty::Bool,
                format!("icmp ule i64 {}, {}", start.text, end.text),
            );
            let len_ok = self.instruction(
                Ty::Bool,
                format!("icmp ule i64 {}, {}", end.text, length.text),
            );
            let bounds_ok = self.instruction(
                Ty::Bool,
                format!("and i1 {}, {}", order_ok.text, len_ok.text),
            );
            let result_type = repr(self.graph, &expr.ty, self.checked)?;
            let absent = self.optional_absent_value(&result_type)?;
            let slot = self.slot(result_type.clone());
            self.store(&slot, &absent)?;
            let check_start = self.fresh();
            let check_start_byte = self.fresh();
            let check_end = self.fresh();
            let check_end_byte = self.fresh();
            let make_slice = self.fresh();
            let done = self.fresh();
            self.line(format!(
                "br i1 {}, label %{check_start}, label %{done}",
                bounds_ok.text
            ));
            self.label(&check_start);
            let address = self.fresh();
            self.line(format!(
                "%{address} = extractvalue {} {}, 0",
                text.ty.llvm(),
                text.text
            ));
            let start_at_end = self.instruction(
                Ty::Bool,
                format!("icmp eq i64 {}, {}", start.text, length.text),
            );
            self.line(format!(
                "br i1 {}, label %{make_slice}, label %{check_start_byte}",
                start_at_end.text
            ));
            self.label(&check_start_byte);
            let start_ptr = self.fresh();
            self.line(format!(
                "%{start_ptr} = getelementptr i8, ptr %{address}, i64 {}",
                start.text
            ));
            let start_byte = self.instruction(
                Ty::Int(8, true),
                format!("load i8, ptr %{start_ptr}, align 1"),
            );
            let start_ok =
                self.instruction(Ty::Bool, format!("icmp sge i8 {}, -64", start_byte.text));
            self.line(format!(
                "br i1 {}, label %{check_end}, label %{done}",
                start_ok.text
            ));
            self.label(&check_end);
            let end_at_end = self.instruction(
                Ty::Bool,
                format!("icmp eq i64 {}, {}", end.text, length.text),
            );
            self.line(format!(
                "br i1 {}, label %{make_slice}, label %{check_end_byte}",
                end_at_end.text
            ));
            self.label(&check_end_byte);
            let end_ptr = self.fresh();
            self.line(format!(
                "%{end_ptr} = getelementptr i8, ptr %{address}, i64 {}",
                end.text
            ));
            let end_byte = self.instruction(
                Ty::Int(8, true),
                format!("load i8, ptr %{end_ptr}, align 1"),
            );
            let end_ok = self.instruction(Ty::Bool, format!("icmp sge i8 {}, -64", end_byte.text));
            self.line(format!(
                "br i1 {}, label %{make_slice}, label %{done}",
                end_ok.text
            ));
            self.label(&make_slice);
            let slice_ptr = self.fresh();
            self.line(format!(
                "%{slice_ptr} = getelementptr i8, ptr %{address}, i64 {}",
                start.text
            ));
            let slice_len = self.instruction(
                Ty::Int(64, false),
                format!("sub i64 {}, {}", end.text, start.text),
            );
            let s0 = self.instruction(
                Ty::String,
                format!("insertvalue {{ ptr, i64 }} undef, ptr %{slice_ptr}, 0"),
            );
            let s1 = self.instruction(
                Ty::String,
                format!(
                    "insertvalue {{ ptr, i64 }} {}, i64 {}, 1",
                    s0.text, slice_len.text
                ),
            );
            let present = self.optional_present_value(&result_type, s1)?;
            self.store(&slot, &present)?;
            self.branch(&done);
            self.label(&done);
            return Ok(self.load(&slot));
        }
        let index = self.record_field(&packet, 1)?;
        let within = self.instruction(
            Ty::Bool,
            format!("icmp ult i64 {}, {}", index.text, length.text),
        );
        let result_type = repr(self.graph, &expr.ty, self.checked)?;
        let absent = self.optional_absent_value(&result_type)?;
        let slot = self.slot(result_type.clone());
        self.store(&slot, &absent)?;
        let found = self.fresh();
        let done = self.fresh();
        self.line(format!(
            "br i1 {}, label %{}, label %{}",
            within.text, found, done
        ));
        self.label(&found);
        // Extract the address as raw LLVM: no source pointer or ownership is created.
        let address = self.fresh();
        self.line(format!(
            "%{address} = extractvalue {} {}, 0",
            text.ty.llvm(),
            text.text
        ));
        let offset = self.fresh();
        self.line(format!(
            "%{offset} = getelementptr i8, ptr %{address}, i64 {}",
            index.text
        ));
        let byte = self.instruction(
            Ty::Int(8, false),
            format!("load i8, ptr %{offset}, align 1"),
        );
        let present = self.optional_present_value(&result_type, byte)?;
        self.store(&slot, &present)?;
        self.branch(&done);
        self.label(&done);
        Ok(self.load(&slot))
    }
}
