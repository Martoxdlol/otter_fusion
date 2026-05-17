/// Central table that maps byte offsets to file/line/column information.
/// Inspired by rustc's SourceMap and Clang's SourceManager.
///
/// The SourceMap is created once when the source file is loaded, and is
/// consulted only when an error needs to be displayed. This means the
/// happy path (no errors) pays zero cost.
pub struct SourceMap {
    file_name: String,
    source: String,
    /// Byte offsets where each line starts (0-indexed internally).
    /// line_starts[0] is always 0.
    line_starts: Vec<u32>,
}

/// Human-readable location resolved from a (line, col) pair.
#[derive(Debug, Clone, PartialEq)]
pub struct LocInfo {
    pub file: String,
    pub line: usize,   // 1-indexed
    pub col: usize,    // 1-indexed
}

impl SourceMap {
    /// Build a SourceMap by scanning the source for newline positions.
    pub fn new(file_name: &str, source: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (i, ch) in source.char_indices() {
            if ch == '\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        SourceMap {
            file_name: file_name.to_string(),
            source: source.to_string(),
            line_starts,
        }
    }

    /// Resolve a (line, col) pair — both 1-indexed — to a LocInfo.
    pub fn loc(&self, line: usize, col: usize) -> LocInfo {
        LocInfo {
            file: self.file_name.clone(),
            line,
            col,
        }
    }

    /// Get the source text of an entire line (1-indexed). Returns empty
    /// string if the line is out of range.
    pub fn line_text(&self, line: usize) -> &str {
        if line == 0 || line > self.line_starts.len() {
            return "";
        }
        let start = self.line_starts[line - 1] as usize;
        let end = if line < self.line_starts.len() {
            self.line_starts[line] as usize
        } else {
            self.source.len()
        };
        // Trim trailing newline
        let text = &self.source[start..end];
        text.trim_end_matches('\n').trim_end_matches('\r')
    }

    /// Format a diagnostic message with a source snippet and caret marker.
    ///
    /// Example output:
    /// ```text
    /// error: unexpected character '@'
    ///   --> main.of:1:5
    ///    |
    ///  1 | var @x = 5;
    ///    |     ^
    /// ```
    pub fn render_error(&self, line: usize, col: usize, message: &str) -> String {
        let loc = self.loc(line, col);
        let line_text = self.line_text(line);
        let line_num_width = format!("{}", line).len();

        let mut out = String::new();
        // Header
        out.push_str(&format!("error: {}\n", message));
        out.push_str(&format!(
            "{:>width$}--> {}:{}:{}\n",
            " ",
            loc.file,
            loc.line,
            loc.col,
            width = line_num_width + 1,
        ));
        // Separator
        out.push_str(&format!("{:>width$} |\n", " ", width = line_num_width + 1));
        // Source line
        out.push_str(&format!(
            "{:>width$} | {}\n",
            line,
            line_text,
            width = line_num_width,
        ));
        // Caret
        out.push_str(&format!(
            "{:>width$} | {:>col$}\n",
            " ",
            "^",
            width = line_num_width,
            col = col,
        ));

        out
    }

    /// Format a diagnostic with a range of columns highlighted.
    pub fn render_error_span(
        &self,
        line: usize,
        col_start: usize,
        col_end: usize,
        message: &str,
    ) -> String {
        let loc = self.loc(line, col_start);
        let line_text = self.line_text(line);
        let line_num_width = format!("{}", line).len();
        let span_len = if col_end > col_start {
            col_end - col_start
        } else {
            1
        };

        let mut out = String::new();
        out.push_str(&format!("error: {}\n", message));
        out.push_str(&format!(
            "{:>width$}--> {}:{}:{}\n",
            " ",
            loc.file,
            loc.line,
            loc.col,
            width = line_num_width + 1,
        ));
        out.push_str(&format!("{:>width$} |\n", " ", width = line_num_width + 1));
        out.push_str(&format!(
            "{:>width$} | {}\n",
            line,
            line_text,
            width = line_num_width,
        ));
        // Underline
        let padding = " ".repeat(col_start - 1);
        let underline = "^".repeat(span_len);
        out.push_str(&format!(
            "{:>width$} | {}{}\n",
            " ",
            padding,
            underline,
            width = line_num_width,
        ));

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_loc() {
        let sm = SourceMap::new("test.of", "var x = 5;");
        let loc = sm.loc(1, 5);
        assert_eq!(loc.file, "test.of");
        assert_eq!(loc.line, 1);
        assert_eq!(loc.col, 5);
    }

    #[test]
    fn line_text_single_line() {
        let sm = SourceMap::new("test.of", "var x = 5;");
        assert_eq!(sm.line_text(1), "var x = 5;");
    }

    #[test]
    fn line_text_multi_line() {
        let src = "line one\nline two\nline three";
        let sm = SourceMap::new("test.of", src);
        assert_eq!(sm.line_text(1), "line one");
        assert_eq!(sm.line_text(2), "line two");
        assert_eq!(sm.line_text(3), "line three");
    }

    #[test]
    fn line_text_out_of_range() {
        let sm = SourceMap::new("test.of", "hello");
        assert_eq!(sm.line_text(0), "");
        assert_eq!(sm.line_text(99), "");
    }

    #[test]
    fn line_text_empty_file() {
        let sm = SourceMap::new("empty.of", "");
        assert_eq!(sm.line_text(1), "");
    }

    #[test]
    fn line_text_trailing_newlines() {
        let sm = SourceMap::new("test.of", "hello\nworld\n");
        assert_eq!(sm.line_text(1), "hello");
        assert_eq!(sm.line_text(2), "world");
    }

    #[test]
    fn render_error_single_caret() {
        let sm = SourceMap::new("main.of", "var @x = 5;");
        let output = sm.render_error(1, 5, "unexpected character '@'");
        assert!(output.contains("error: unexpected character '@'"));
        assert!(output.contains("--> main.of:1:5"));
        assert!(output.contains("var @x = 5;"));
        assert!(output.contains("^"));
    }

    #[test]
    fn render_error_multiline_source() {
        let src = "function test() {\n  var x = ;\n}";
        let sm = SourceMap::new("main.of", src);
        let output = sm.render_error(2, 11, "unexpected token ';'");
        assert!(output.contains("--> main.of:2:11"));
        assert!(output.contains("var x = ;"));
    }

    #[test]
    fn render_error_span_underline() {
        let src = "function test(): str {\n  42\n}";
        let sm = SourceMap::new("main.of", src);
        let output = sm.render_error_span(2, 3, 5, "expected str, found i64");
        assert!(output.contains("error: expected str, found i64"));
        assert!(output.contains("^^"));
    }
}
