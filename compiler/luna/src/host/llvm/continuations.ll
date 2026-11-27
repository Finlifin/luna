; Pooled-stack capturing delimited continuation runtime.
; Status values: 0 = RUNNING, 1 = SUSPENDED, 2 = COMPLETED, 3 = UNWIND_PANIC, 4 = ABORTING
%flurry.cont = type { ptr, ptr, i64, ptr, ptr, i32, ptr, ptr, ptr, ptr, i64, ptr, i64, ptr, ptr, ptr, i8, i64, i64, %flurry.task, ptr, ptr, i64, ptr }

declare ptr @flurry_ctx_switch(ptr, ptr, ptr)
declare void @flurry_ctx_trampoline()

@flurry_active_cont = internal thread_local global ptr null, align 8
@flurry_c_frame_depth = internal thread_local global i64 0, align 8
@flurry_stack_pool_head = internal thread_local global ptr null, align 8
@flurry_stack_pool_count = internal thread_local global i64 0, align 8

define internal ptr @flurry_stack_acquire(ptr %pool) {
entry:
  %head = load ptr, ptr @flurry_stack_pool_head, align 8
  %has_cached = icmp ne ptr %head, null
  br i1 %has_cached, label %pop, label %alloc

pop:
  %next = load ptr, ptr %head, align 8
  store ptr %next, ptr @flurry_stack_pool_head, align 8
  %cnt = load i64, ptr @flurry_stack_pool_count, align 8
  %dec = sub i64 %cnt, 1
  store i64 %dec, ptr @flurry_stack_pool_count, align 8
  ret ptr %head

alloc:
  %slot = alloca ptr, align 8
  store ptr null, ptr %slot, align 8
  %rc = call i32 @posix_memalign(ptr %slot, i64 4096, i64 262144)
  %ok = icmp eq i32 %rc, 0
  br i1 %ok, label %check_ptr, label %oom

check_ptr:
  %mem = load ptr, ptr %slot, align 8
  %nonnull = icmp ne ptr %mem, null
  br i1 %nonnull, label %done, label %oom

done:
  ret ptr %mem

oom:
  call void @llvm.trap()
  unreachable
}

define internal void @flurry_stack_release(ptr %pool, ptr %base, i64 %bytes) {
entry:
  %is_null = icmp eq ptr %base, null
  br i1 %is_null, label %ret_void, label %check_cap

check_cap:
  %cnt = load i64, ptr @flurry_stack_pool_count, align 8
  %can_cache = icmp ult i64 %cnt, 16
  br i1 %can_cache, label %push, label %free_slot

push:
  %head = load ptr, ptr @flurry_stack_pool_head, align 8
  store ptr %head, ptr %base, align 8
  store ptr %base, ptr @flurry_stack_pool_head, align 8
  %inc = add i64 %cnt, 1
  store i64 %inc, ptr @flurry_stack_pool_count, align 8
  ret void

free_slot:
  call void @free(ptr %base)
  ret void

ret_void:
  ret void
}

define internal void @flurry_cont_entry(ptr %cont) {
entry:
  %entry_fn_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 14
  %entry_fn = load ptr, ptr %entry_fn_p, align 8
  %entry_env_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 15
  %entry_env = load ptr, ptr %entry_env_p, align 8
  %b_storage_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 11
  %b_storage = load ptr, ptr %b_storage_p, align 8
  %task_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 19
  %res = call { {}, ptr } %entry_fn(ptr %entry_env, ptr %b_storage, ptr %task_p)
  %ctrl = extractvalue { {}, ptr } %res, 1
  %status_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 5
  %cur_status = load i32, ptr %status_p, align 4
  %is_aborting = icmp eq i32 %cur_status, 4
  br i1 %is_aborting, label %finish_switch, label %check_ctrl

check_ctrl:
  %ok = icmp eq ptr %ctrl, null
  br i1 %ok, label %mark_done, label %mark_unwind

mark_done:
  store i32 2, ptr %status_p, align 4
  %ctrl_p0 = getelementptr %flurry.cont, ptr %cont, i32 0, i32 13
  store ptr null, ptr %ctrl_p0, align 8
  br label %finish_switch

mark_unwind:
  store i32 3, ptr %status_p, align 4
  %ctrl_p1 = getelementptr %flurry.cont, ptr %cont, i32 0, i32 13
  store ptr %ctrl, ptr %ctrl_p1, align 8
  br label %finish_switch

finish_switch:
  %parent_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 20
  %parent = load ptr, ptr %parent_p, align 8
  store ptr %parent, ptr @flurry_active_cont, align 8
  %sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 0
  %caller_sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 3
  %caller_sp = load ptr, ptr %caller_sp_p, align 8
  %unused = call ptr @flurry_ctx_switch(ptr %sp_p, ptr %caller_sp, ptr null)
  call void @llvm.trap()
  unreachable
}

define internal i1 @flurry_cont_owns_handler(ptr %cont, ptr %hf, ptr %he) {
entry:
  %n_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 22
  %n = load i64, ptr %n_p, align 8
  %arr_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 21
  %arr = load ptr, ptr %arr_p, align 8
  br label %loop

loop:
  %i = phi i64 [ 0, %entry ], [ %next_i, %next ]
  %done = icmp uge i64 %i, %n
  br i1 %done, label %not_found, label %check

check:
  %slot = getelementptr { ptr, ptr }, ptr %arr, i64 %i
  %fn_p = getelementptr { ptr, ptr }, ptr %slot, i32 0, i32 0
  %fn_val = load ptr, ptr %fn_p, align 8
  %fn_eq = icmp eq ptr %fn_val, %hf
  br i1 %fn_eq, label %check_env, label %next

check_env:
  %env_p = getelementptr { ptr, ptr }, ptr %slot, i32 0, i32 1
  %env_val = load ptr, ptr %env_p, align 8
  %env_eq = icmp eq ptr %env_val, %he
  br i1 %env_eq, label %found, label %next

next:
  %next_i = add i64 %i, 1
  br label %loop

found:
  ret i1 true

not_found:
  ret i1 false
}

define internal i8 @flurry_cont_suspend(ptr %handler_fn, ptr %handler_env, ptr %args_ptr, ptr %y_ptr, i64 %y_bytes) {
entry:
  %active = load ptr, ptr @flurry_active_cont, align 8
  %has_active = icmp ne ptr %active, null
  br i1 %has_active, label %check_raw, label %no_cont

no_cont:
  call void @llvm.trap()
  unreachable

check_raw:
  %raw_n_p = getelementptr %flurry.cont, ptr %active, i32 0, i32 22
  %raw_n = load i64, ptr %raw_n_p, align 8
  %is_raw = icmp eq i64 %raw_n, 0
  br i1 %is_raw, label %save, label %find_loop

find_loop:
  %cur = phi ptr [ %active, %check_raw ], [ %cur_parent, %step_parent ]
  %cur_nonnull = icmp ne ptr %cur, null
  br i1 %cur_nonnull, label %check_owner, label %no_cont

check_owner:
  %owns = call i1 @flurry_cont_owns_handler(ptr %cur, ptr %handler_fn, ptr %handler_env)
  br i1 %owns, label %save, label %step_parent

step_parent:
  %cur_parent_p = getelementptr %flurry.cont, ptr %cur, i32 0, i32 20
  %cur_parent = load ptr, ptr %cur_parent_p, align 8
  br label %find_loop

save:
  %cont = phi ptr [ %active, %check_raw ], [ %cur, %check_owner ]
  %cur_c_depth = load i64, ptr @flurry_c_frame_depth, align 8
  %start_c_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 17
  %start_c = load i64, ptr %start_c_p, align 8
  %crossed = icmp ugt i64 %cur_c_depth, %start_c
  br i1 %crossed, label %mark_c, label %store_fields

mark_c:
  %c_flag_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 16
  store i8 1, ptr %c_flag_p, align 1
  br label %store_fields

store_fields:
  %status_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 5
  store i32 1, ptr %status_p, align 4
  %hf_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 6
  store ptr %handler_fn, ptr %hf_p, align 8
  %he_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 7
  store ptr %handler_env, ptr %he_p, align 8
  %ap_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 8
  store ptr %args_ptr, ptr %ap_p, align 8
  %rp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 9
  store ptr %y_ptr, ptr %rp_p, align 8
  %yb_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 10
  store i64 %y_bytes, ptr %yb_p, align 8
  %ra_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 23
  store ptr %active, ptr %ra_p, align 8
  %parent_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 20
  %parent = load ptr, ptr %parent_p, align 8
  store ptr %parent, ptr @flurry_active_cont, align 8
  %sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 0
  %caller_sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 3
  %caller_sp = load ptr, ptr %caller_sp_p, align 8
  %unused = call ptr @flurry_ctx_switch(ptr %sp_p, ptr %caller_sp, ptr null)
  %new_status = load i32, ptr %status_p, align 4
  %aborting = icmp eq i32 %new_status, 4
  %res = zext i1 %aborting to i8
  ret i8 %res
}

define internal void @flurry_cont_free(ptr %cont) {
entry:
  %base_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 1
  %base = load ptr, ptr %base_p, align 8
  %bytes_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 2
  %bytes = load i64, ptr %bytes_p, align 8
  %pool_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 4
  %pool = load ptr, ptr %pool_p, align 8
  call void @flurry_stack_release(ptr %pool, ptr %base, i64 %bytes)
  %b_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 11
  %b_buf = load ptr, ptr %b_p, align 8
  %has_b = icmp ne ptr %b_buf, null
  br i1 %has_b, label %free_b, label %free_cont

free_b:
  call void @free(ptr %b_buf)
  br label %free_cont

free_cont:
  call void @free(ptr %cont)
  ret void
}

define internal void @flurry_cont_sync_task(ptr %cont, ptr %caller_task) {
entry:
  %has_caller = icmp ne ptr %caller_task, null
  br i1 %has_caller, label %check_cancel, label %done

check_cancel:
  %polled = call ptr @flurry_task_poll(ptr %caller_task)
  %cancelled = icmp ne ptr %polled, null
  br i1 %cancelled, label %propagate, label %done

propagate:
  %inner_task = getelementptr %flurry.cont, ptr %cont, i32 0, i32 19
  call void @flurry_task_cancel(ptr %inner_task)
  br label %done

done:
  ret void
}

define internal ptr @flurry_cont_dispatch_loop(ptr %cont, ptr %out_b_ptr, ptr %caller_task) {
entry:
  br label %loop_head

loop_head:
  %status_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 5
  %status = load i32, ptr %status_p, align 4
  %is_susp = icmp eq i32 %status, 1
  br i1 %is_susp, label %dispatch, label %finished

dispatch:
  %hf_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 6
  %hf = load ptr, ptr %hf_p, align 8
  %he_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 7
  %he = load ptr, ptr %he_p, align 8
  %ap_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 8
  %ap = load ptr, ptr %ap_p, align 8
  %rp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 9
  %rp = load ptr, ptr %rp_p, align 8
  %resumed_slot = alloca i8, align 1
  store i8 0, ptr %resumed_slot, align 1
  %h_res = call { {}, ptr } %hf(ptr %he, ptr %ap, ptr %cont, ptr %rp, ptr %out_b_ptr, ptr %resumed_slot, ptr %caller_task)
  %h_ctrl = extractvalue { {}, ptr } %h_res, 1
  %resumed = load i8, ptr %resumed_slot, align 1
  %did_resume = icmp ne i8 %resumed, 0
  br i1 %did_resume, label %resume_cont, label %arm_done

resume_cont:
  %h_ok = icmp eq ptr %h_ctrl, null
  br i1 %h_ok, label %do_switch, label %unexpected_ctrl

unexpected_ctrl:
  call void @llvm.trap()
  unreachable

do_switch:
  store i32 0, ptr %status_p, align 4
  call void @flurry_cont_sync_task(ptr %cont, ptr %caller_task)
  %prev_active = load ptr, ptr @flurry_active_cont, align 8
  %parent_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 20
  store ptr %prev_active, ptr %parent_p, align 8
  %ra_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 23
  %ra = load ptr, ptr %ra_p, align 8
  %has_ra = icmp ne ptr %ra, null
  %next_active = select i1 %has_ra, ptr %ra, ptr %cont
  store ptr %next_active, ptr @flurry_active_cont, align 8
  %caller_sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 3
  %sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 0
  %sp = load ptr, ptr %sp_p, align 8
  %unused = call ptr @flurry_ctx_switch(ptr %caller_sp_p, ptr %sp, ptr null)
  br label %loop_head

arm_done:
  ; The arm did not tail-resume: handler_fn owns %cont (or moved/aborted it).
  ; Never dereference %cont after handler_fn returns resumed == 0.
  ret ptr %h_ctrl

finished:
  %ctrl_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 13
  %ctrl = load ptr, ptr %ctrl_p, align 8
  %ok = icmp eq ptr %ctrl, null
  br i1 %ok, label %copy_b, label %cleanup_cont

copy_b:
  %b_bytes_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 12
  %b_bytes = load i64, ptr %b_bytes_p, align 8
  %has_bytes = icmp ugt i64 %b_bytes, 0
  %has_out = icmp ne ptr %out_b_ptr, null
  %do_copy = and i1 %has_bytes, %has_out
  br i1 %do_copy, label %memcpy_b, label %cleanup_cont

memcpy_b:
  %b_buf_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 11
  %b_buf = load ptr, ptr %b_buf_p, align 8
  call void @llvm.memcpy.p0.p0.i64(ptr %out_b_ptr, ptr %b_buf, i64 %b_bytes, i1 false)
  br label %cleanup_cont

cleanup_cont:
  call void @flurry_cont_free(ptr %cont)
  ret ptr %ctrl
}

define internal ptr @flurry_cap_run_with_handlers(ptr %entry_fn, ptr %entry_env, ptr %out_b_ptr, i64 %b_bytes, ptr %own_handlers, i64 %own_count, ptr %caller_task) {
entry:
  %cont_sz_p = getelementptr %flurry.cont, ptr null, i32 1
  %cont_sz = ptrtoint ptr %cont_sz_p to i64
  %cont = call ptr @malloc(i64 %cont_sz)
  %cont_ok = icmp ne ptr %cont, null
  br i1 %cont_ok, label %init, label %oom

oom:
  call void @llvm.trap()
  unreachable

init:
  %pool = call ptr @flurry_task_pool(ptr %caller_task)
  %base = call ptr @flurry_stack_acquire(ptr %pool)
  %b_nz = icmp ugt i64 %b_bytes, 0
  br i1 %b_nz, label %alloc_b, label %no_b

alloc_b:
  %b_mem = call ptr @malloc(i64 %b_bytes)
  %b_ok = icmp ne ptr %b_mem, null
  br i1 %b_ok, label %setup, label %oom

no_b:
  br label %setup

setup:
  %b_storage = phi ptr [ %b_mem, %alloc_b ], [ null, %no_b ]
  %init_sp = call ptr @flurry_cont_init_frame(ptr %base, i64 262144, ptr @flurry_cont_entry)
  %sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 0
  store ptr %init_sp, ptr %sp_p, align 8
  %base_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 1
  store ptr %base, ptr %base_p, align 8
  %bytes_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 2
  store i64 262144, ptr %bytes_p, align 8
  %pool_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 4
  store ptr %pool, ptr %pool_p, align 8
  %status_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 5
  store i32 0, ptr %status_p, align 4
  %b_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 11
  store ptr %b_storage, ptr %b_p, align 8
  %bb_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 12
  store i64 %b_bytes, ptr %bb_p, align 8
  %ctrl_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 13
  store ptr null, ptr %ctrl_p, align 8
  %ef_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 14
  store ptr %entry_fn, ptr %ef_p, align 8
  %ee_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 15
  store ptr %entry_env, ptr %ee_p, align 8
  %cf_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 16
  store i8 0, ptr %cf_p, align 1
  %c_depth = load i64, ptr @flurry_c_frame_depth, align 8
  %cd_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 17
  store i64 %c_depth, ptr %cd_p, align 8
  %tid_i = ptrtoint ptr @flurry_c_frame_depth to i64
  %tid_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 18
  store i64 %tid_i, ptr %tid_p, align 8
  %inner_task = getelementptr %flurry.cont, ptr %cont, i32 0, i32 19
  call void @flurry_task_init(ptr %inner_task, ptr %pool)
  call void @flurry_cont_sync_task(ptr %cont, ptr %caller_task)
  %oh_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 21
  store ptr %own_handlers, ptr %oh_p, align 8
  %oc_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 22
  store i64 %own_count, ptr %oc_p, align 8
  %ra_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 23
  store ptr %cont, ptr %ra_p, align 8
  %prev_active = load ptr, ptr @flurry_active_cont, align 8
  %parent_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 20
  store ptr %prev_active, ptr %parent_p, align 8
  store ptr %cont, ptr @flurry_active_cont, align 8
  %caller_sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 3
  %unused = call ptr @flurry_ctx_switch(ptr %caller_sp_p, ptr %init_sp, ptr %cont)
  %ctrl = call ptr @flurry_cont_dispatch_loop(ptr %cont, ptr %out_b_ptr, ptr %caller_task)
  ret ptr %ctrl
}

define internal ptr @flurry_cap_run(ptr %entry_fn, ptr %entry_env, ptr %out_b_ptr, i64 %b_bytes, ptr %caller_task) {
entry:
  %ctrl = call ptr @flurry_cap_run_with_handlers(ptr %entry_fn, ptr %entry_env, ptr %out_b_ptr, i64 %b_bytes, ptr null, i64 0, ptr %caller_task)
  ret ptr %ctrl
}

define internal i64 @flurry_cont_resume_raw(ptr %cont, i64 %arg_addr, i64 %out_addr, ptr %caller_task) {
entry:
  %nonnull = icmp ne ptr %cont, null
  br i1 %nonnull, label %check_status, label %bad

check_status:
  %status_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 5
  %status = load i32, ptr %status_p, align 4
  %is_susp = icmp eq i32 %status, 1
  br i1 %is_susp, label %copy_y, label %bad

bad:
  call void @llvm.trap()
  unreachable

copy_y:
  %yb_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 10
  %y_bytes = load i64, ptr %yb_p, align 8
  %has_y = icmp ugt i64 %y_bytes, 0
  br i1 %has_y, label %do_copy_y, label %switch_in

do_copy_y:
  %arg_ptr = inttoptr i64 %arg_addr to ptr
  %arg_ok = icmp ne ptr %arg_ptr, null
  br i1 %arg_ok, label %memcpy_y, label %bad

memcpy_y:
  %rp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 9
  %rp = load ptr, ptr %rp_p, align 8
  call void @llvm.memcpy.p0.p0.i64(ptr %rp, ptr %arg_ptr, i64 %y_bytes, i1 false)
  br label %switch_in

switch_in:
  store i32 0, ptr %status_p, align 4
  call void @flurry_cont_sync_task(ptr %cont, ptr %caller_task)
  %prev_active = load ptr, ptr @flurry_active_cont, align 8
  %parent_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 20
  store ptr %prev_active, ptr %parent_p, align 8
  %ra_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 23
  %ra = load ptr, ptr %ra_p, align 8
  %has_ra = icmp ne ptr %ra, null
  %next_active = select i1 %has_ra, ptr %ra, ptr %cont
  store ptr %next_active, ptr @flurry_active_cont, align 8
  %caller_sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 3
  %sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 0
  %sp = load ptr, ptr %sp_p, align 8
  %unused = call ptr @flurry_ctx_switch(ptr %caller_sp_p, ptr %sp, ptr null)
  %out_ptr = inttoptr i64 %out_addr to ptr
  %ctrl = call ptr @flurry_cont_dispatch_loop(ptr %cont, ptr %out_ptr, ptr %caller_task)
  %ctrl_i = ptrtoint ptr %ctrl to i64
  ret i64 %ctrl_i
}

define internal void @flurry_cont_abort(ptr %cont, ptr %caller_task) {
entry:
  %is_null = icmp eq ptr %cont, null
  br i1 %is_null, label %done, label %check_status

check_status:
  %status_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 5
  %status = load i32, ptr %status_p, align 4
  %is_susp = icmp eq i32 %status, 1
  br i1 %is_susp, label %check_c, label %bad

check_c:
  %cf_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 16
  %cf = load i8, ptr %cf_p, align 1
  %crosses = icmp ne i8 %cf, 0
  br i1 %crosses, label %bad, label %abort_switch

bad:
  call void @llvm.trap()
  unreachable

abort_switch:
  store i32 4, ptr %status_p, align 4
  call void @flurry_cont_sync_task(ptr %cont, ptr %caller_task)
  %prev_active = load ptr, ptr @flurry_active_cont, align 8
  %parent_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 20
  store ptr %prev_active, ptr %parent_p, align 8
  %ra_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 23
  %ra = load ptr, ptr %ra_p, align 8
  %has_ra = icmp ne ptr %ra, null
  %next_active = select i1 %has_ra, ptr %ra, ptr %cont
  store ptr %next_active, ptr @flurry_active_cont, align 8
  %caller_sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 3
  %sp_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 0
  %sp = load ptr, ptr %sp_p, align 8
  %unused = call ptr @flurry_ctx_switch(ptr %caller_sp_p, ptr %sp, ptr null)
  call void @flurry_cont_free(ptr %cont)
  br label %done

done:
  ret void
}

define internal i1 @flurry_cont_crosses_c_frame(ptr %cont) {
entry:
  %is_null = icmp eq ptr %cont, null
  br i1 %is_null, label %ret_false, label %load_flag

load_flag:
  %cf_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 16
  %cf = load i8, ptr %cf_p, align 1
  %res = icmp ne i8 %cf, 0
  ret i1 %res

ret_false:
  ret i1 false
}

define internal i64 @flurry_cont_origin_thread_id(ptr %cont) {
entry:
  %is_null = icmp eq ptr %cont, null
  br i1 %is_null, label %ret_zero, label %load_tid

load_tid:
  %tid_p = getelementptr %flurry.cont, ptr %cont, i32 0, i32 18
  %tid = load i64, ptr %tid_p, align 8
  ret i64 %tid

ret_zero:
  ret i64 0
}
