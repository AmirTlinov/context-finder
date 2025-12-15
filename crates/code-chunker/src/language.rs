use crate::error::{ChunkerError, Result};
use std::path::Path;

/// Supported programming language
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Ruby,
    Swift,
    Kotlin,
    Markdown,
    Yaml,
    Json,
    Config,
    Sql,
    Shell,
    Terraform,
    Html,
    Css,
    Unknown,
}

impl Language {
    /// Detect language from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "rs" => Self::Rust,
            "py" | "pyw" => Self::Python,
            "js" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "tsx" => Self::TypeScript,
            "go" => Self::Go,
            "java" => Self::Java,
            "c" | "h" => Self::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Self::Cpp,
            "cs" => Self::CSharp,
            "rb" => Self::Ruby,
            "swift" => Self::Swift,
            "kt" | "kts" => Self::Kotlin,
            "md" | "mdx" | "rst" | "adoc" => Self::Markdown,
            "yaml" | "yml" => Self::Yaml,
            "json" => Self::Json,
            "toml" | "ini" | "cfg" | "conf" | "properties" | "env" | "gradle" | "groovy" => {
                Self::Config
            }
            "sql" | "dbml" => Self::Sql,
            "sh" | "bash" | "zsh" | "fish" | "ps1" | "bat" | "cmd" => Self::Shell,
            "tf" | "tfvars" | "hcl" => Self::Terraform,
            "html" => Self::Html,
            "css" | "scss" | "less" => Self::Css,
            _ => Self::Unknown,
        }
    }

    /// Detect language from file path
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        path.as_ref()
            .extension()
            .and_then(|ext| ext.to_str())
            .map(Self::from_extension)
            .or_else(|| {
                path.as_ref()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| match name {
                        "Dockerfile" | "docker-compose.yml" => Some(Self::Terraform),
                        "Makefile" | "makefile" => Some(Self::Shell),
                        _ => None,
                    })
            })
            .unwrap_or(Self::Unknown)
    }

    /// Get language name as string
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Go => "go",
            Self::Java => "java",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::CSharp => "csharp",
            Self::Ruby => "ruby",
            Self::Swift => "swift",
            Self::Kotlin => "kotlin",
            Self::Markdown => "markdown",
            Self::Yaml => "yaml",
            Self::Json => "json",
            Self::Config => "config",
            Self::Sql => "sql",
            Self::Shell => "shell",
            Self::Terraform => "terraform",
            Self::Html => "html",
            Self::Css => "css",
            Self::Unknown => "unknown",
        }
    }

    /// Check if this language is supported for AST parsing
    pub const fn supports_ast(self) -> bool {
        matches!(
            self,
            Self::Rust | Self::Python | Self::JavaScript | Self::TypeScript
        )
    }

    /// Get Tree-sitter language instance
    pub fn tree_sitter_language(self) -> Result<tree_sitter::Language> {
        match self {
            Self::Rust => Ok(tree_sitter_rust::LANGUAGE.into()),
            Self::Python => Ok(tree_sitter_python::LANGUAGE.into()),
            Self::JavaScript => Ok(tree_sitter_javascript::LANGUAGE.into()),
            Self::TypeScript => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            _ => Err(ChunkerError::unsupported_language(self.as_str())),
        }
    }

    /// Get typical comment prefixes for this language
    pub fn comment_prefixes(self) -> Vec<&'static str> {
        match self {
            Self::Rust
            | Self::JavaScript
            | Self::TypeScript
            | Self::Go
            | Self::Java
            | Self::C
            | Self::Cpp
            | Self::CSharp
            | Self::Swift
            | Self::Kotlin => vec!["//", "/*", "///", "/**"],
            Self::Python | Self::Ruby => vec!["#", "\"\"\"", "'''"],
            Self::Markdown => vec![">", "#"],
            Self::Yaml | Self::Config | Self::Shell => vec!["#"],
            Self::Json => vec!["//"],
            Self::Sql => vec!["--", "/*"],
            Self::Terraform => vec!["#", "//"],
            Self::Html | Self::Css => vec!["<!--", "/*"],
            Self::Unknown => vec![],
        }
    }

    /// Get import/use statement patterns for this language
    pub fn import_patterns(self) -> Vec<&'static str> {
        match self {
            Self::Rust => vec!["use ", "extern crate "],
            Self::Python => vec!["import ", "from "],
            Self::JavaScript | Self::TypeScript => vec!["import ", "require("],
            Self::Go | Self::Java | Self::Swift | Self::Kotlin => vec!["import "],
            Self::CSharp => vec!["using "],
            Self::Ruby => vec!["require ", "include "],
            Self::C | Self::Cpp => vec!["#include "],
            Self::Yaml
            | Self::Json
            | Self::Config
            | Self::Sql
            | Self::Shell
            | Self::Terraform
            | Self::Markdown
            | Self::Html
            | Self::Css
            | Self::Unknown => vec![],
        }
    }

    /// Get typical file size thresholds for this language
    pub const fn size_limits(self) -> LanguageSizeLimits {
        match self {
            Self::Python | Self::Ruby => LanguageSizeLimits {
                typical_lines: 200,
                large_lines: 500,
                huge_lines: 1000,
            },
            Self::Rust | Self::Go => LanguageSizeLimits {
                typical_lines: 300,
                large_lines: 600,
                huge_lines: 1200,
            },
            Self::JavaScript | Self::TypeScript => LanguageSizeLimits {
                typical_lines: 150,
                large_lines: 400,
                huge_lines: 800,
            },
            Self::Markdown | Self::Yaml | Self::Json | Self::Config => LanguageSizeLimits {
                typical_lines: 200,
                large_lines: 500,
                huge_lines: 1200,
            },
            Self::Sql => LanguageSizeLimits {
                typical_lines: 150,
                large_lines: 300,
                huge_lines: 700,
            },
            Self::Shell | Self::Terraform => LanguageSizeLimits {
                typical_lines: 120,
                large_lines: 300,
                huge_lines: 600,
            },
            _ => LanguageSizeLimits {
                typical_lines: 250,
                large_lines: 500,
                huge_lines: 1000,
            },
        }
    }
}

/// Size thresholds for language files
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_field_names)]
pub struct LanguageSizeLimits {
    pub typical_lines: usize,
    pub large_lines: usize,
    pub huge_lines: usize,
}

#[cfg(test)]
mod tests {
    use super::Language;

    #[test]
    fn test_from_extension() {
        assert_eq!(Language::from_extension("rs"), Language::Rust);
        assert_eq!(Language::from_extension("RS"), Language::Rust);
        assert_eq!(Language::from_extension("py"), Language::Python);
        assert_eq!(Language::from_extension("js"), Language::JavaScript);
        assert_eq!(Language::from_extension("ts"), Language::TypeScript);
        assert_eq!(Language::from_extension("md"), Language::Markdown);
        assert_eq!(Language::from_extension("yaml"), Language::Yaml);
        assert_eq!(Language::from_extension("json"), Language::Json);
        assert_eq!(Language::from_extension("toml"), Language::Config);
        assert_eq!(Language::from_extension("sql"), Language::Sql);
        assert_eq!(Language::from_extension("unknown"), Language::Unknown);
    }

    #[test]
    fn test_from_path() {
        assert_eq!(Language::from_path("test.rs"), Language::Rust);
        assert_eq!(Language::from_path("src/main.py"), Language::Python);
        assert_eq!(Language::from_path("index.ts"), Language::TypeScript);
        assert_eq!(Language::from_path("no_extension"), Language::Unknown);
        assert_eq!(Language::from_path("README.md"), Language::Markdown);
        assert_eq!(Language::from_path("Dockerfile"), Language::Terraform);
    }

    #[test]
    fn test_supports_ast() {
        assert!(Language::Rust.supports_ast());
        assert!(Language::Python.supports_ast());
        assert!(Language::JavaScript.supports_ast());
        assert!(Language::TypeScript.supports_ast());
        assert!(!Language::Go.supports_ast());
        assert!(!Language::Unknown.supports_ast());
    }

    #[test]
    fn test_tree_sitter_language() {
        assert!(Language::Rust.tree_sitter_language().is_ok());
        assert!(Language::Python.tree_sitter_language().is_ok());
        assert!(Language::JavaScript.tree_sitter_language().is_ok());
        assert!(Language::TypeScript.tree_sitter_language().is_ok());
        assert!(Language::Go.tree_sitter_language().is_err());
    }

    #[test]
    fn test_comment_prefixes() {
        assert!(!Language::Rust.comment_prefixes().is_empty());
        assert!(Language::Rust.comment_prefixes().contains(&"//"));
        assert!(Language::Python.comment_prefixes().contains(&"#"));
    }

    #[test]
    fn test_import_patterns() {
        assert!(Language::Rust.import_patterns().contains(&"use "));
        assert!(Language::Python.import_patterns().contains(&"import "));
        assert!(Language::JavaScript.import_patterns().contains(&"import "));
    }
}
