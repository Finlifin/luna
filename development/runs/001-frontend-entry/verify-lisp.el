;;; verify-lisp.el --- Check an IR dump with a real Lisp reader/formatter -*- lexical-binding: t; -*-

;; Run: emacs --batch -Q --script verify-lisp.el dump.lisp
(let ((path (pop command-line-args-left)))
  (unless path (error "Expected a dump path"))
  (with-temp-buffer
    (insert-file-contents path)
    (goto-char (point-min))
    (let ((form (read (current-buffer))))
      (unless (eq (car-safe form) 'hir-package)
        (error "Expected a hir-package form"))
      (skip-chars-forward " \t\r\n")
      (unless (eobp) (error "Unexpected trailing content"))
      (emacs-lisp-mode)
      (indent-region (point-min) (point-max))
      (goto-char (point-min))
      (unless (equal form (read (current-buffer)))
        (error "Formatting changed the S-expression"))
      (message "PASS: one HIR S-expression; Emacs indentation preserves its structure"))))
