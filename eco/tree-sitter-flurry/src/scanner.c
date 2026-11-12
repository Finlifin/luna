#include "tree_sitter/parser.h"
#include <stdbool.h>
#include <stdint.h>
#include <wctype.h>

enum TokenType {
  BLOCK_COMMENT,
  SEP_PLUS,
  SEP_MINUS,
  SEP_STAR,
  SEP_SLASH,
  SEP_PERCENT,
  SEP_LT,
  SEP_GT,
  TIGHT_LT,
  TIGHT_GT,
  TAKE_VIEW_TICK,
  CHAR_LITERAL,
  MACRO_CONTENT,
  STRING_CONTENT,
  RAW_BLOCK_CONTENT,
  ERROR_SENTINEL,
};

static inline bool is_ident_start(int32_t c) {
  return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_' ||
         iswalpha(c) || (c >= 0x2600 && c <= 0x27BF) ||
         (c >= 0x1F000 && c <= 0x1F9FF);
}

static inline bool is_space(int32_t c) {
  return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f' ||
         c == '\v' || (c > 127 && iswspace(c));
}

void *tree_sitter_flurry_external_scanner_create(void) { return NULL; }

void tree_sitter_flurry_external_scanner_destroy(void *payload) {
  (void)payload;
}

unsigned tree_sitter_flurry_external_scanner_serialize(void *payload,
                                                       char *buffer) {
  (void)payload;
  (void)buffer;
  return 0;
}

void tree_sitter_flurry_external_scanner_deserialize(void *payload,
                                                     const char *buffer,
                                                     unsigned length) {
  (void)payload;
  (void)buffer;
  (void)length;
}

static bool scan_block_comment_after_open_brace(TSLexer *lexer) {
  // Caller has already consumed '{' and verified lookahead == '-'.
  lexer->advance(lexer, false); // consume '-'
  int depth = 1;
  while (lexer->lookahead != 0 && depth > 0) {
    if (lexer->lookahead == '{') {
      lexer->advance(lexer, false);
      if (lexer->lookahead == '-') {
        lexer->advance(lexer, false);
        depth++;
      }
    } else if (lexer->lookahead == '-') {
      lexer->advance(lexer, false);
      if (lexer->lookahead == '}') {
        lexer->advance(lexer, false);
        depth--;
      }
    } else {
      lexer->advance(lexer, false);
    }
  }
  lexer->mark_end(lexer);
  lexer->result_symbol = BLOCK_COMMENT;
  return true;
}

static bool scan_string_content(TSLexer *lexer) {
  bool has_content = false;
  while (lexer->lookahead != 0 && lexer->lookahead != '"' &&
         lexer->lookahead != '\\' && lexer->lookahead != '\n') {
    if (lexer->lookahead == '$') {
      lexer->mark_end(lexer);
      lexer->advance(lexer, false);
      if (lexer->lookahead == '{') {
        if (has_content) {
          lexer->result_symbol = STRING_CONTENT;
          return true;
        }
        return false;
      }
      has_content = true;
      lexer->mark_end(lexer);
      continue;
    }
    lexer->advance(lexer, false);
    has_content = true;
    lexer->mark_end(lexer);
  }
  if (has_content) {
    lexer->result_symbol = STRING_CONTENT;
    return true;
  }
  return false;
}

static bool scan_raw_block_content(TSLexer *lexer) {
  bool has_content = false;
  int depth = 0;
  while (lexer->lookahead != 0) {
    if (lexer->lookahead == '}' && depth == 0) {
      break;
    }
    if (lexer->lookahead == '{') {
      depth++;
    } else if (lexer->lookahead == '}') {
      depth--;
    }
    lexer->advance(lexer, false);
    has_content = true;
  }
  if (has_content) {
    lexer->mark_end(lexer);
    lexer->result_symbol = RAW_BLOCK_CONTENT;
    return true;
  }
  return false;
}

static bool scan_quote_or_char(TSLexer *lexer, const bool *valid_symbols,
                               bool had_whitespace) {
  // Consume opening '\''
  lexer->advance(lexer, false);
  lexer->mark_end(lexer);

  if (!had_whitespace && valid_symbols[TAKE_VIEW_TICK]) {
    lexer->result_symbol = TAKE_VIEW_TICK;
    return true;
  }

  // Check closed character literal: '\...' or '<single-scalar>'
  if (valid_symbols[CHAR_LITERAL]) {
    if (lexer->lookahead == '\\') {
      lexer->advance(lexer, false);
      if (lexer->lookahead == 'x') {
        lexer->advance(lexer, false);
        if (lexer->lookahead == '{') {
          lexer->advance(lexer, false);
          while (lexer->lookahead != 0 && lexer->lookahead != '}' &&
                 lexer->lookahead != '\n') {
            lexer->advance(lexer, false);
          }
          if (lexer->lookahead == '}') {
            lexer->advance(lexer, false);
          }
        }
      } else if (lexer->lookahead != 0 && lexer->lookahead != '\n') {
        lexer->advance(lexer, false);
      }
      if (lexer->lookahead == '\'') {
        lexer->advance(lexer, false);
        lexer->mark_end(lexer);
        lexer->result_symbol = CHAR_LITERAL;
        return true;
      }
      return false;
    }

    if (lexer->lookahead != 0 && lexer->lookahead != '\'' &&
        lexer->lookahead != '\n') {
      int32_t first = lexer->lookahead;
      lexer->advance(lexer, false);
      if (lexer->lookahead == '\'') {
        lexer->advance(lexer, false);
        lexer->mark_end(lexer);
        lexer->result_symbol = CHAR_LITERAL;
        return true;
      }
      // Not a closed char literal; check macro content '{ ... }
      int32_t cur = first;
      if (is_space(cur)) {
        while (is_space(lexer->lookahead)) {
          lexer->advance(lexer, false);
        }
        cur = lexer->lookahead;
        if (cur == '{') {
          lexer->advance(lexer, false);
        }
      }
      if (cur == '{' && valid_symbols[MACRO_CONTENT]) {
        int depth = 1;
        while (lexer->lookahead != 0 && depth > 0) {
          if (lexer->lookahead == '{') {
            depth++;
          } else if (lexer->lookahead == '}') {
            depth--;
          }
          lexer->advance(lexer, false);
        }
        if (depth == 0) {
          lexer->mark_end(lexer);
          lexer->result_symbol = MACRO_CONTENT;
          return true;
        }
      }
      if (valid_symbols[TAKE_VIEW_TICK] && is_ident_start(first)) {
        lexer->result_symbol = TAKE_VIEW_TICK;
        return true;
      }
      return false;
    }
  } else {
    // Even when CHAR_LITERAL is not valid, check MACRO_CONTENT or TAKE_VIEW_TICK
    while (is_space(lexer->lookahead)) {
      lexer->advance(lexer, false);
    }
    if (lexer->lookahead == '{' && valid_symbols[MACRO_CONTENT]) {
      lexer->advance(lexer, false);
      int depth = 1;
      while (lexer->lookahead != 0 && depth > 0) {
        if (lexer->lookahead == '{') {
          depth++;
        } else if (lexer->lookahead == '}') {
          depth--;
        }
        lexer->advance(lexer, false);
      }
      if (depth == 0) {
        lexer->mark_end(lexer);
        lexer->result_symbol = MACRO_CONTENT;
        return true;
      }
    }
    if (valid_symbols[TAKE_VIEW_TICK] && is_ident_start(lexer->lookahead)) {
      lexer->result_symbol = TAKE_VIEW_TICK;
      return true;
    }
  }

  return false;
}

bool tree_sitter_flurry_external_scanner_scan(void *payload, TSLexer *lexer,
                                              const bool *valid_symbols) {
  (void)payload;

  if (!valid_symbols[ERROR_SENTINEL]) {
    if (valid_symbols[STRING_CONTENT] && scan_string_content(lexer)) {
      return true;
    }
    if (valid_symbols[RAW_BLOCK_CONTENT] && scan_raw_block_content(lexer)) {
      return true;
    }
  }

  bool had_whitespace = false;
  while (is_space(lexer->lookahead)) {
    had_whitespace = true;
    lexer->advance(lexer, true);
  }

  if (lexer->lookahead == 0) {
    return false;
  }

  if (lexer->lookahead == '{' && valid_symbols[BLOCK_COMMENT]) {
    lexer->advance(lexer, false);
    if (lexer->lookahead == '-') {
      return scan_block_comment_after_open_brace(lexer);
    }
    return false;
  }

  if (valid_symbols[ERROR_SENTINEL]) {
    return false;
  }

  switch (lexer->lookahead) {
  case '+':
    if (valid_symbols[SEP_PLUS] && had_whitespace) {
      lexer->advance(lexer, false);
      if (lexer->lookahead != '=' && lexer->lookahead != '+' &&
          is_space(lexer->lookahead)) {
        lexer->mark_end(lexer);
        lexer->result_symbol = SEP_PLUS;
        return true;
      }
    }
    return false;

  case '-':
    if (valid_symbols[SEP_MINUS] && had_whitespace) {
      lexer->advance(lexer, false);
      if (lexer->lookahead != '=' && lexer->lookahead != '>' &&
          lexer->lookahead != '-' && lexer->lookahead != '}' &&
          is_space(lexer->lookahead)) {
        lexer->mark_end(lexer);
        lexer->result_symbol = SEP_MINUS;
        return true;
      }
    }
    return false;

  case '*':
    if (valid_symbols[SEP_STAR] && had_whitespace) {
      lexer->advance(lexer, false);
      if (lexer->lookahead != '=' && is_space(lexer->lookahead)) {
        lexer->mark_end(lexer);
        lexer->result_symbol = SEP_STAR;
        return true;
      }
    }
    return false;

  case '/':
    if (valid_symbols[SEP_SLASH] && had_whitespace) {
      lexer->advance(lexer, false);
      if (lexer->lookahead != '=' && is_space(lexer->lookahead)) {
        lexer->mark_end(lexer);
        lexer->result_symbol = SEP_SLASH;
        return true;
      }
    }
    return false;

  case '%':
    if (valid_symbols[SEP_PERCENT] && had_whitespace) {
      lexer->advance(lexer, false);
      if (lexer->lookahead != '=' && is_space(lexer->lookahead)) {
        lexer->mark_end(lexer);
        lexer->result_symbol = SEP_PERCENT;
        return true;
      }
    }
    return false;

  case '<':
    if (valid_symbols[SEP_LT] || valid_symbols[TIGHT_LT]) {
      lexer->advance(lexer, false);
      if (lexer->lookahead == '=' || lexer->lookahead == ':') {
        return false;
      }
      bool sep = had_whitespace && is_space(lexer->lookahead);
      if (sep && valid_symbols[SEP_LT]) {
        lexer->mark_end(lexer);
        lexer->result_symbol = SEP_LT;
        return true;
      }
      if (!sep && valid_symbols[TIGHT_LT]) {
        lexer->mark_end(lexer);
        lexer->result_symbol = TIGHT_LT;
        return true;
      }
    }
    return false;

  case '>':
    if (valid_symbols[SEP_GT] || valid_symbols[TIGHT_GT]) {
      lexer->advance(lexer, false);
      if (lexer->lookahead == '=') {
        return false;
      }
      bool sep = had_whitespace && is_space(lexer->lookahead);
      if (sep && valid_symbols[SEP_GT]) {
        lexer->mark_end(lexer);
        lexer->result_symbol = SEP_GT;
        return true;
      }
      if (!sep && valid_symbols[TIGHT_GT]) {
        lexer->mark_end(lexer);
        lexer->result_symbol = TIGHT_GT;
        return true;
      }
      if (sep && valid_symbols[TIGHT_GT]) {
        lexer->mark_end(lexer);
        lexer->result_symbol = TIGHT_GT;
        return true;
      }
    }
    return false;

  case '\'':
    return scan_quote_or_char(lexer, valid_symbols, had_whitespace);

  default:
    return false;
  }
}
