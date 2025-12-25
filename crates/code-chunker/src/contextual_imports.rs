use crate::language::Language;
use std::collections::HashSet;

pub fn extract_identifiers_from_import(language: Language, import: &str) -> Vec<String> {
    let mut identifiers = Vec::new();

    match language {
        Language::Rust => {
            // use std::collections::HashMap -> HashMap
            // use crate::error::{Result, Error} -> Result, Error
            if let Some(last_part) = import.split("::").last() {
                // Handle {A, B, C}
                if let Some(inner) = last_part.trim().strip_prefix('{') {
                    let inner = inner.trim_end().trim_end_matches('}');
                    for ident in inner.split(',') {
                        let ident = ident.trim();
                        if !ident.is_empty() {
                            identifiers.push(ident.to_string());
                        }
                    }
                } else {
                    let ident = last_part.trim();
                    if !ident.is_empty() {
                        identifiers.push(ident.to_string());
                    }
                }
            }
        }
        Language::Python => {
            // from x import A, B -> A, B
            // import x -> x
            if import.contains("import") {
                if let Some(after_import) = import.split("import").nth(1) {
                    for ident in after_import.split(',') {
                        let ident = ident.trim();
                        if !ident.is_empty() {
                            identifiers.push(ident.to_string());
                        }
                    }
                }
            }
        }
        Language::JavaScript | Language::TypeScript => {
            // import { A, B } from 'x' -> A, B
            if let (Some(inner_start), Some(inner_end)) = (import.find('{'), import.find('}')) {
                if inner_end > inner_start + 1 {
                    let inner = &import[inner_start + 1..inner_end];
                    for ident in inner.split(',') {
                        let ident = ident.trim();
                        if !ident.is_empty() {
                            identifiers.push(ident.to_string());
                        }
                    }
                }
            }
        }
        _ => {}
    }

    identifiers
}

pub fn filter_relevant_imports(
    language: Language,
    file_imports: &[String],
    code_content: &str,
    limit: usize,
) -> Vec<String> {
    if limit == 0 || file_imports.is_empty() || code_content.is_empty() {
        return Vec::new();
    }

    // For languages we don't have a good identifier heuristic for yet, fall back
    // to the first N imports (still bounded).
    if !matches!(
        language,
        Language::Rust | Language::Python | Language::JavaScript | Language::TypeScript
    ) {
        return file_imports.iter().take(limit).cloned().collect();
    }

    let mut relevant = Vec::new();
    for import in file_imports {
        let identifiers = extract_identifiers_from_import(language, import);
        if identifiers
            .iter()
            .any(|ident| !ident.is_empty() && code_content.contains(ident))
        {
            relevant.push(import.clone());
        }

        if relevant.len() >= limit {
            break;
        }
    }

    relevant
}

pub fn extract_imports_from_lines(language: Language, lines: &[&str], limit: usize) -> Vec<String> {
    if limit == 0 || lines.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for line in lines {
        if out.len() >= limit {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if !is_import_line(language, trimmed) {
            continue;
        }

        // Keep closer to the AST import extraction format:
        // - remove trailing semicolons
        // - keep only the first line
        let cleaned = trimmed
            .trim_end_matches(';')
            .lines()
            .next()
            .unwrap_or(trimmed)
            .trim()
            .to_string();
        if cleaned.is_empty() {
            continue;
        }

        if seen.insert(cleaned.clone()) {
            out.push(cleaned);
        }
    }

    out
}

fn is_import_line(language: Language, line: &str) -> bool {
    match language {
        Language::Rust => {
            let line = line.strip_prefix("pub ").unwrap_or(line);
            line.starts_with("use ") || line.starts_with("extern crate ")
        }
        Language::Python => line.starts_with("import ") || line.starts_with("from "),
        Language::JavaScript | Language::TypeScript => {
            line.starts_with("import ") || line.contains("require(")
        }
        Language::Go | Language::Java | Language::Swift | Language::Kotlin => {
            line.starts_with("import ")
        }
        Language::CSharp => line.starts_with("using "),
        Language::Ruby => line.starts_with("require ") || line.starts_with("include "),
        Language::C | Language::Cpp => line.starts_with("#include "),
        _ => false,
    }
}
