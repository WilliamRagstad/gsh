use clap::ColorChoice;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const UNDERLINE: &str = "\x1b[4m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";

#[derive(Debug, Clone)]
struct TableCell {
    text: String,
    visible_len: usize,
}

impl TableCell {
    fn new(text: String, visible_len: usize) -> Self {
        Self { text, visible_len }
    }
}

pub fn print_table(headers: &[&str], rows: &[Vec<String>], color_choice: ColorChoice) {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i >= widths.len() {
                widths.push(cell.len());
            } else {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    let header_cells: Vec<TableCell> = headers
        .iter()
        .map(|h| TableCell::new(styled_header(h, color_choice), h.len()))
        .collect();
    println!("{}", format_table_row(&header_cells, &widths));

    for row in rows {
        let cells: Vec<TableCell> = row
            .iter()
            .enumerate()
            .map(|(i, v)| TableCell::new(styled_value(i, v, color_choice), v.len()))
            .collect();
        println!("{}", format_table_row(&cells, &widths));
    }
}

pub fn fingerprints_summary(fingerprints: &[Vec<u8>]) -> String {
    if fingerprints.is_empty() {
        return "-".to_string();
    }

    let first = hex_fingerprint_short(&fingerprints[0], 16);
    if fingerprints.len() == 1 {
        first
    } else {
        format!("{} (+{})", first, fingerprints.len() - 1)
    }
}

fn format_table_row(cells: &[TableCell], widths: &[usize]) -> String {
    let mut out = String::new();
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.push_str("  ");
        }
        let width = widths.get(i).copied().unwrap_or(0);
        out.push_str(&cell.text);
        let padding = width.saturating_sub(cell.visible_len);
        for _ in 0..padding {
            out.push(' ');
        }
    }
    out
}

fn styled_header(text: &str, color_choice: ColorChoice) -> String {
    if matches!(color_choice, ColorChoice::Never) {
        return text.to_string();
    }
    format!("{BOLD}{UNDERLINE}{YELLOW}{text}{RESET}")
}

fn styled_value(col: usize, text: &str, color_choice: ColorChoice) -> String {
    if matches!(color_choice, ColorChoice::Never) {
        return text.to_string();
    }

    if text == "-" {
        return format!("{DIM}{text}{RESET}");
    }

    let color = match col {
        0 => CYAN,
        1 => GREEN,
        _ => YELLOW,
    };
    format!("{color}{text}{RESET}")
}

fn hex_fingerprint_short(bytes: &[u8], max_bytes: usize) -> String {
    let take = bytes.len().min(max_bytes);
    let mut parts = Vec::with_capacity(take);
    for b in &bytes[..take] {
        parts.push(format!("{:02X}", b));
    }
    let mut s = parts.join(":");
    if bytes.len() > take {
        s.push_str(":...");
    }
    s
}
