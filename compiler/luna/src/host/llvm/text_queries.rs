use super::*;

impl Emitter<'_> {
    pub(super) fn text_query(
        &mut self,
        expr: &Expr,
        subject: &Expr,
        query: nir::program::TextQueryKind,
        depth: usize,
    ) -> Result<Value, String> {
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
        let index = self.record_field(&packet, 1)?;
        let within = self.instruction(
            Ty::Bool,
            format!("icmp ult i64 {}, {}", index.text, length.text),
        );
        let result_type = repr(self.graph, &expr.ty, self.checked)?;
        let slot = self.slot(result_type.clone());
        self.store(
            &slot,
            &Value {
                ty: result_type.clone(),
                text: "zeroinitializer".into(),
            },
        )?;
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
        let present = self.instruction(
            result_type.clone(),
            format!(
                "insertvalue {} zeroinitializer, i1 true, 0",
                result_type.llvm()
            ),
        );
        let present = self.instruction(
            result_type.clone(),
            format!(
                "insertvalue {} {}, i8 {}, 1",
                result_type.llvm(),
                present.text,
                byte.text
            ),
        );
        self.store(&slot, &present)?;
        self.branch(&done);
        self.label(&done);
        Ok(self.load(&slot))
    }
}
