; ModuleID = 'luna_module'
source_filename = "luna_module"

%Node = type { i64, ptr }

declare i32 @printf(ptr, ...)

define void @main() {
entry:
  %_1 = alloca %Node, align 8
  %_2 = alloca %Node, align 8
  %_3 = alloca %Node, align 8
  %_4 = alloca ptr, align 8
  %_5 = alloca %Node, align 8
  br label %bb0

bb0:                                              ; preds = %entry
  store %Node { i64 1, ptr null }, ptr %_2, align 8
  %load = load %Node, ptr %_2, align 8
  store %Node %load, ptr %_1, align 8
  store ptr %_1, ptr %_4, align 8
  %load1 = load ptr, ptr %_4, align 8
  %field = insertvalue %Node { i64 2, ptr undef }, ptr %load1, 1
  store %Node %field, ptr %_5, align 8
  %load2 = load %Node, ptr %_5, align 8
  store %Node %load2, ptr %_3, align 8
  ret void
}
