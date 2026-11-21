; The stack owner keeps its context alive while a task has suspended frames.
%flurry.task = type { i8, ptr }

define void @flurry_task_init(ptr nonnull %task, ptr %pool) {
entry:
  %owner = getelementptr %flurry.task, ptr %task, i32 0, i32 1
  store ptr %pool, ptr %owner
  store atomic i8 0, ptr %task release, align 1
  ret void
}

define void @flurry_task_cancel(ptr nonnull %task) {
entry:
  store atomic i8 1, ptr %task release, align 1
  ret void
}

define ptr @flurry_task_poll(ptr nonnull %task) {
entry:
  %flag = load atomic i8, ptr %task acquire, align 1
  %cancel = icmp ne i8 %flag, 0
  %control = select i1 %cancel, ptr %task, ptr null
  ret ptr %control
}

define ptr @flurry_task_pool(ptr nonnull %task) {
entry:
  %owner = getelementptr %flurry.task, ptr %task, i32 0, i32 1
  %pool = load ptr, ptr %owner
  ret ptr %pool
}
