#!/usr/bin/env python3
"""Repository Rust formatting wrapper enforcing item-level blank-line spacing.

Runs `rustfmt --config-path <root>/rustfmt.toml` followed by a Rust-lexer-aware
pass that inserts a single blank line between adjacent Rust items inside files,
`mod { ... }`, `impl { ... }`, `trait { ... }`, and `extern { ... }` containers
while leaving attributes, doc comments, grouped `use`/`mod_decl`/`extern crate`
lists, and function/struct/enum/macro bodies intact.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import subprocess
import sys
import tomllib

CONTAINER_ITEM_KINDS = frozenset({"mod_block", "impl", "trait", "extern_block"})
EXEMPT_GROUP_KINDS = frozenset({"use", "extern_crate"})
QUALIFIER_WORDS = frozenset({"pub", "unsafe", "async", "default", "auto"})
SEMICOLON_ONLY_KINDS = frozenset({"use", "const", "static", "type", "extern_crate"})
NON_BODY_PREV_TOKENS = frozenset({"=", "<", ",", "(", "[", "+", ":", ";", "const"})


@dataclass(frozen=True)
class Token:
    kind: str
    text: str
    line: int
    end_line: int


@dataclass(frozen=True)
class ItemSpan:
    start_line: int
    end_line: int
    kind: str
    has_outer_attr: bool


@dataclass
class _ActiveItem:
    start_line: int
    container_depth: int
    has_outer_attr: bool
    header_kind: str | None = None
    seen_top_eq_or_colon: bool = False
    waiting_macro_bang: bool = False
    body_brace_depth: int | None = None
    pending_macro_brace: bool = False


def load_ignored_paths(root: Path) -> set[Path]:
    config_path = root / "rustfmt.toml"
    if not config_path.is_file():
        return set()
    data = tomllib.loads(config_path.read_text(encoding="utf-8"))
    raw_ignored = data.get("ignore", [])
    ignored: set[Path] = set()
    for entry in raw_ignored:
        candidate = (root / str(entry)).resolve()
        ignored.add(candidate)
    return ignored


def _is_ident_start(ch: str) -> bool:
    return ch == "_" or ch.isalpha()


def _is_ident_continue(ch: str) -> bool:
    return ch == "_" or ch.isalnum()


def _tokenize_rust(source: str) -> list[Token]:
    tokens: list[Token] = []
    n = len(source)
    i = 0
    line = 0

    def advance_lines(chunk: str, start_line: int) -> int:
        return start_line + chunk.count("\n")

    while i < n:
        ch = source[i]

        if ch in " \t\r\n\v\f":
            if ch == "\n":
                line += 1
            i += 1
            continue

        # Line comments
        if source.startswith("//", i):
            nl = source.find("\n", i)
            end = n if nl == -1 else nl
            text = source[i:end]
            if text.startswith("//!"):
                kind = "COMMENT_INNER_LINE"
            elif text.startswith("///") and not text.startswith("////"):
                kind = "COMMENT_OUTER_LINE"
            else:
                kind = "COMMENT_LINE"
            tokens.append(Token(kind, text, line, line))
            i = end
            continue

        # Nested block comments
        if source.startswith("/*", i):
            start_i = i
            start_line = line
            depth = 1
            i += 2
            while i < n and depth > 0:
                if source.startswith("/*", i):
                    depth += 1
                    i += 2
                elif source.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    if source[i] == "\n":
                        line += 1
                    i += 1
            text = source[start_i:i]
            if text.startswith("/*!"):
                kind = "COMMENT_INNER_BLOCK"
            elif text.startswith("/**") and not text.startswith("/***") and text != "/**/":
                kind = "COMMENT_OUTER_BLOCK"
            else:
                kind = "COMMENT_BLOCK"
            tokens.append(Token(kind, text, start_line, line))
            continue

        # Raw strings / byte raw strings / c raw strings: (r|br|cr)#*"..."#*
        raw_prefix_len = 0
        for prefix in ("br", "cr", "r"):
            if source.startswith(prefix, i):
                j = i + len(prefix)
                hashes = 0
                while j + hashes < n and source[j + hashes] == "#":
                    hashes += 1
                if j + hashes < n and source[j + hashes] == '"':
                    raw_prefix_len = len(prefix) + hashes + 1
                    closing = '"' + ("#" * hashes)
                    start_line = line
                    k = i + raw_prefix_len
                    while k < n:
                        if source.startswith(closing, k):
                            k += len(closing)
                            break
                        if source[k] == "\n":
                            line += 1
                        k += 1
                    tokens.append(Token("STRING", source[i:k], start_line, line))
                    i = k
                    break
        if raw_prefix_len:
            continue

        # Raw identifiers: r#ident
        if source.startswith("r#", i) and i + 2 < n and _is_ident_start(source[i + 2]):
            j = i + 2
            while j < n and _is_ident_continue(source[j]):
                j += 1
            tokens.append(Token("RAW_IDENT", source[i:j], line, line))
            i = j
            continue

        # Normal / byte / c strings: "...", b"...", c"..."
        if ch == '"' or (source.startswith(('b"', 'c"'), i)):
            start_i = i
            start_line = line
            i += 1 if ch == '"' else 2
            while i < n:
                c = source[i]
                if c == "\\":
                    if i + 1 < n:
                        if source[i + 1] == "\n":
                            line += 1
                        i += 2
                        continue
                    i += 1
                    break
                if c == "\n":
                    line += 1
                i += 1
                if c == '"':
                    break
            tokens.append(Token("STRING", source[start_i:i], start_line, line))
            continue

        # Byte character literal: b'...'
        if source.startswith("b'", i):
            start_i = i
            i += 2
            if i < n and source[i] == "\\":
                i += 2
                while i < n and source[i] not in ("'", "\n"):
                    i += 1
                if i < n and source[i] == "'":
                    i += 1
            else:
                while i < n and source[i] not in ("'", "\n"):
                    i += 1
                if i < n and source[i] == "'":
                    i += 1
            tokens.append(Token("STRING", source[start_i:i], line, line))
            continue

        # Character literal vs lifetime/label
        if ch == "'":
            start_i = i
            if i + 1 < n and source[i + 1] == "\\":
                start_line = line
                i += 2
                while i < n and source[i] not in ("'", "\n"):
                    i += 1
                if i < n and source[i] == "'":
                    i += 1
                tokens.append(Token("STRING", source[start_i:i], start_line, line))
                continue
            if i + 2 < n and source[i + 2] == "'" and source[i + 1] not in ("\n", "\r", "\\"):
                tokens.append(Token("STRING", source[i : i + 3], line, line))
                i += 3
                continue
            if source.startswith("'r#", i) and i + 3 < n and _is_ident_start(source[i + 3]):
                j = i + 3
                while j < n and _is_ident_continue(source[j]):
                    j += 1
                tokens.append(Token("LIFETIME", source[i:j], line, line))
                i = j
                continue
            j = i + 1
            while j < n and _is_ident_continue(source[j]):
                j += 1
            tokens.append(Token("LIFETIME", source[i:j], line, line))
            i = j
            continue

        # Identifiers / numbers
        if _is_ident_start(ch):
            j = i + 1
            while j < n and _is_ident_continue(source[j]):
                j += 1
            tokens.append(Token("IDENT", source[i:j], line, line))
            i = j
            continue

        if ch.isdigit():
            j = i + 1
            while j < n and (source[j].isalnum() or source[j] == "_"):
                j += 1
            tokens.append(Token("NUMBER", source[i:j], line, line))
            i = j
            continue

        if source.startswith("::", i):
            tokens.append(Token("PUNCT", "::", line, line))
            i += 2
            continue

        tokens.append(Token("PUNCT", ch, line, line))
        i += 1

    return tokens


def _classify_line_kind(line_text: str) -> str:
    stripped = line_text.strip()
    if not stripped:
        return "blank"
    if stripped.startswith("//!"):
        return "inner_comment"
    if stripped.startswith("//"):
        return "comment"
    if stripped.startswith("/*!"):
        return "inner_comment"
    if stripped.startswith("/*"):
        return "comment"
    return "code"


def _attach_leading_comment_start(
    lines: list[str],
    item_start: int,
    prev_end: int,
    in_block_comment_line: set[int],
) -> int:
    idx = item_start - 1
    attached_start = item_start
    while idx > prev_end:
        stripped = lines[idx].strip()
        if not stripped:
            break
        if stripped.startswith(("//! ", "//!", "/*!")) or stripped == "//!":
            break
        if stripped.startswith(("//", "/*")) or idx in in_block_comment_line:
            attached_start = idx
            idx -= 1
            continue
        break
    return attached_start


def ensure_blank_lines_between_items(source: str) -> str:
    if not source:
        return source

    lines = source.splitlines()
    had_trailing_newline = source.endswith("\n")
    tokens = _tokenize_rust(source)

    non_trivia = [t for t in tokens if not t.kind.startswith("COMMENT_")]
    block_comment_lines: set[int] = set()
    for t in tokens:
        if t.kind in ("COMMENT_BLOCK", "COMMENT_OUTER_BLOCK"):
            for ln in range(t.line, t.end_line + 1):
                block_comment_lines.add(ln)

    brace_stack: list[bool] = []
    container_items: dict[int, list[ItemSpan]] = {0: []}
    container_owner: dict[int, _ActiveItem] = {}

    paren_depth = 0
    bracket_depth = 0
    in_inner_attr = False
    in_outer_attr = False
    active: _ActiveItem | None = None
    insert_before_lines: set[int] = set()

    def in_item_container() -> bool:
        return all(brace_stack)

    prev_tok_text = ""
    total = len(non_trivia)

    for idx, tok in enumerate(non_trivia):
        next_tok = non_trivia[idx + 1] if idx + 1 < total else None

        if in_inner_attr:
            if tok.kind == "PUNCT" and tok.text == "[":
                bracket_depth += 1
            elif tok.kind == "PUNCT" and tok.text == "]":
                bracket_depth -= 1
                if bracket_depth <= 0:
                    bracket_depth = 0
                    in_inner_attr = False
            elif tok.kind == "PUNCT" and tok.text == "{":
                brace_stack.append(False)
            elif tok.kind == "PUNCT" and tok.text == "}":
                if brace_stack:
                    brace_stack.pop()
            prev_tok_text = tok.text
            continue

        if (
            active is None
            and in_item_container()
            and paren_depth == 0
            and bracket_depth == 0
            and tok.kind == "PUNCT"
            and tok.text == "#"
            and next_tok is not None
            and next_tok.kind == "PUNCT"
            and next_tok.text == "!"
        ):
            in_inner_attr = True
            prev_tok_text = tok.text
            continue

        if (
            in_item_container()
            and paren_depth == 0
            and bracket_depth == 0
            and active is None
            and not (tok.kind == "PUNCT" and tok.text in ("}", ";", ")", "]"))
        ):
            is_attr = tok.kind == "PUNCT" and tok.text == "#"
            active = _ActiveItem(
                start_line=tok.line,
                container_depth=len(brace_stack),
                has_outer_attr=is_attr,
            )
            if is_attr:
                in_outer_attr = True

        # Track parentheses and brackets cleanly
        if tok.kind == "PUNCT" and tok.text == "(":
            paren_depth += 1
        elif tok.kind == "PUNCT" and tok.text == ")":
            paren_depth = max(0, paren_depth - 1)
        elif tok.kind == "PUNCT" and tok.text == "[":
            bracket_depth += 1
        elif tok.kind == "PUNCT" and tok.text == "]":
            bracket_depth = max(0, bracket_depth - 1)
            if bracket_depth == 0 and in_outer_attr:
                in_outer_attr = False

        if (
            active is not None
            and len(brace_stack) == active.container_depth
            and paren_depth == 0
            and bracket_depth == 0
            and not in_outer_attr
        ):
            # Track header keyword classification carefully
            if tok.kind == "IDENT":
                word = tok.text
                if active.header_kind is None:
                    if word == "mod":
                        active.header_kind = "mod_block"
                    elif word == "impl":
                        active.header_kind = "impl"
                    elif word == "trait":
                        active.header_kind = "trait"
                    elif word == "extern":
                        active.header_kind = "extern_block"
                    elif word == "use":
                        active.header_kind = "use"
                    elif word == "fn":
                        active.header_kind = "fn"
                    elif word == "struct":
                        active.header_kind = "struct"
                    elif word == "enum":
                        active.header_kind = "enum"
                    elif word == "union":
                        active.header_kind = "union"
                    elif word == "const":
                        active.header_kind = "const"
                    elif word == "static":
                        active.header_kind = "static"
                    elif word == "type":
                        active.header_kind = "type"
                    elif word == "macro_rules":
                        active.header_kind = "macro"
                    elif word not in QUALIFIER_WORDS and not active.seen_top_eq_or_colon:
                        active.waiting_macro_bang = True
                elif active.header_kind == "const" and not active.seen_top_eq_or_colon:
                    if word == "fn":
                        active.header_kind = "fn"
                    elif word == "extern":
                        active.header_kind = "extern_block"
                    elif word == "trait":
                        active.header_kind = "trait"
                elif active.header_kind == "extern_block" and not active.seen_top_eq_or_colon:
                    if word == "crate":
                        active.header_kind = "extern_crate"
                    elif word == "fn":
                        active.header_kind = "fn"
                elif active.header_kind == "macro" and active.pending_macro_brace:
                    pass
                else:
                    active.waiting_macro_bang = False
            elif tok.kind == "PUNCT" and tok.text == "!":
                if active.header_kind is None and active.waiting_macro_bang:
                    active.header_kind = "macro"
                active.waiting_macro_bang = False
            else:
                active.waiting_macro_bang = False

            if tok.kind == "PUNCT" and tok.text in ("=", ":"):
                active.seen_top_eq_or_colon = True
            elif tok.kind == "PUNCT" and tok.text == "#" and not in_outer_attr:
                # Subsequent outer attribute on the same item
                if next_tok is not None and next_tok.text == "[":
                    active.has_outer_attr = True
                    in_outer_attr = True

        # Handle braces and semicolons
        if tok.kind == "PUNCT" and tok.text == "{":
            opens_container = (
                active is not None
                and in_item_container()
                and len(brace_stack) == active.container_depth
                and paren_depth == 0
                and bracket_depth == 0
                and not in_outer_attr
                and prev_tok_text not in NON_BODY_PREV_TOKENS
                and active.header_kind in CONTAINER_ITEM_KINDS
            )
            if opens_container and active is not None:
                brace_stack.append(True)
                new_depth = len(brace_stack)
                container_items[new_depth] = []
                container_owner[new_depth] = active
                active = None
                prev_tok_text = tok.text
                continue

            is_item_body_brace = (
                active is not None
                and len(brace_stack) == active.container_depth
                and paren_depth == 0
                and bracket_depth == 0
                and not in_outer_attr
                and active.body_brace_depth is None
                and active.header_kind not in SEMICOLON_ONLY_KINDS
                and (
                    (active.header_kind == "macro" and prev_tok_text != "")
                    or prev_tok_text not in NON_BODY_PREV_TOKENS
                )
            )
            brace_stack.append(False)
            if is_item_body_brace and active is not None:
                if active.header_kind != "macro" or prev_tok_text == "!" or next_tok is not None:
                    # For macro calls like `foo! { ... }`, body closes item without trailing `;`.
                    # For `const` / `type` / `static` / `use`, `;` closes the item.
                    active.body_brace_depth = len(brace_stack)
                    if active.header_kind == "macro":
                        active.pending_macro_brace = True
            prev_tok_text = tok.text
            continue

        if tok.kind == "PUNCT" and tok.text == "}":
            was_container = brace_stack.pop() if brace_stack else False
            depth_after = len(brace_stack)
            if was_container:
                owner = container_owner.pop(depth_after + 1, None)
                items_in_container = container_items.pop(depth_after + 1, [])
                _collect_insertions(lines, items_in_container, block_comment_lines, insert_before_lines)
                if owner is not None:
                    container_items.setdefault(depth_after, []).append(
                        ItemSpan(
                            owner.start_line,
                            tok.end_line,
                            owner.header_kind or "other",
                            owner.has_outer_attr,
                        )
                    )
            elif (
                active is not None
                and active.body_brace_depth is not None
                and depth_after < active.body_brace_depth
            ):
                container_items.setdefault(active.container_depth, []).append(
                    ItemSpan(
                        active.start_line,
                        tok.end_line,
                        active.header_kind or "other",
                        active.has_outer_attr,
                    )
                )
                active = None
            prev_tok_text = tok.text
            continue

        if (
            tok.kind == "PUNCT"
            and tok.text == ";"
            and active is not None
            and len(brace_stack) == active.container_depth
            and paren_depth == 0
            and bracket_depth == 0
            and not in_outer_attr
            and active.body_brace_depth is None
        ):
            kind = active.header_kind or "other"
            if kind == "mod_block":
                kind = "mod_decl"
            container_items.setdefault(active.container_depth, []).append(
                ItemSpan(
                    active.start_line,
                    tok.end_line,
                    kind,
                    active.has_outer_attr,
                )
            )
            active = None

        prev_tok_text = tok.text

    _collect_insertions(
        lines,
        container_items.get(0, []),
        block_comment_lines,
        insert_before_lines,
    )

    if not insert_before_lines:
        return source

    output_lines: list[str] = []
    for idx, line_text in enumerate(lines):
        if idx in insert_before_lines:
            output_lines.append("")
        output_lines.append(line_text)
    rendered = "\n".join(output_lines)
    if had_trailing_newline:
        rendered += "\n"
    return rendered


def _collect_insertions(
    lines: list[str],
    items: list[ItemSpan],
    block_comment_lines: set[int],
    insert_before_lines: set[int],
) -> None:
    for prev, curr in zip(items, items[1:]):
        if prev.kind in EXEMPT_GROUP_KINDS and curr.kind == prev.kind:
            continue
        curr_block_start = _attach_leading_comment_start(
            lines,
            curr.start_line,
            prev.end_line,
            block_comment_lines,
        )
        if (
            prev.kind == "mod_decl"
            and curr.kind == "mod_decl"
            and not curr.has_outer_attr
            and curr_block_start == curr.start_line
        ):
            continue
        if curr_block_start <= prev.end_line:
            continue
        has_blank = any(
            not lines[ln].strip() for ln in range(prev.end_line + 1, curr_block_start)
        )
        if not has_blank:
            insert_before_lines.add(curr_block_start)


def _expand_paths(root: Path, raw_paths: list[str], ignored: set[Path]) -> list[Path]:
    result: list[Path] = []
    seen: set[Path] = set()
    for raw in raw_paths:
        path = Path(raw)
        if not path.is_absolute():
            path = root / path
        if path.is_dir():
            candidates = sorted(path.rglob("*.rs"))
        elif path.is_file() and path.suffix == ".rs":
            candidates = [path]
        else:
            candidates = []
        for candidate in candidates:
            resolved = candidate.resolve()
            if resolved in ignored or resolved in seen:
                continue
            seen.add(resolved)
            result.append(resolved)
    return result


def format_files(root: Path, files: list[Path], *, check: bool = False) -> tuple[int, str]:
    ignored = load_ignored_paths(root)
    eligible: list[Path] = []
    seen: set[Path] = set()
    for path in files:
        resolved = (path if path.is_absolute() else root / path).resolve()
        if resolved in ignored or resolved in seen or not resolved.is_file():
            continue
        if resolved.suffix != ".rs":
            continue
        seen.add(resolved)
        eligible.append(resolved)

    if not eligible:
        return 0, ""

    config_path = root / "rustfmt.toml"
    if config_path.is_file():
        base_cmd = ["rustfmt", "--config-path", str(config_path)]
    else:
        base_cmd = ["rustfmt", "--edition", "2024"]

    if check:
        proc = subprocess.run(
            [*base_cmd, "--check", *(str(p) for p in eligible)],
            cwd=root,
            capture_output=True,
            text=True,
        )
        unspaced: list[str] = []
        for path in eligible:
            text = path.read_text(encoding="utf-8")
            if ensure_blank_lines_between_items(text) != text:
                try:
                    display = str(path.relative_to(root))
                except ValueError:
                    display = str(path)
                unspaced.append(display)
        if proc.returncode != 0 or unspaced:
            messages: list[str] = []
            if proc.stdout.strip():
                messages.append(proc.stdout.strip())
            if proc.stderr.strip():
                messages.append(proc.stderr.strip())
            if unspaced:
                messages.append(
                    "missing blank line between Rust items in: " + ", ".join(unspaced)
                )
            return proc.returncode if proc.returncode != 0 else 1, "\n".join(messages)
        return 0, ""

    proc = subprocess.run(
        [*base_cmd, *(str(p) for p in eligible)],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout or "").strip()
        return proc.returncode, detail

    changed: list[Path] = []
    for path in eligible:
        original = path.read_text(encoding="utf-8")
        updated = ensure_blank_lines_between_items(original)
        if updated != original:
            path.write_text(updated, encoding="utf-8")
            changed.append(path)

    if changed:
        proc2 = subprocess.run(
            [*base_cmd, *(str(p) for p in changed)],
            cwd=root,
            capture_output=True,
            text=True,
        )
        if proc2.returncode != 0:
            detail = (proc2.stderr or proc2.stdout or "").strip()
            return proc2.returncode, detail

    return 0, ""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Verify formatting without editing files")
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("paths", nargs="+", help="Rust files or directories to format")
    args = parser.parse_args(argv)

    root = args.root.resolve()
    ignored = load_ignored_paths(root)
    targets = _expand_paths(root, args.paths, ignored)
    status, message = format_files(root, targets, check=args.check)
    if message:
        print(message, file=sys.stderr, flush=True)
    return status


if __name__ == "__main__":
    raise SystemExit(main())
