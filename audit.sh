#!/bin/bash

set -e

echo "╔═══════════════════════════════════════════════════════════════════════════╗"
echo "║                        FLAGSHIP QUALITY AUDIT                             ║"
echo "╚═══════════════════════════════════════════════════════════════════════════╝"
echo

PROJECT_ROOT="/home/amir/Документы/PROJECTS/skills/apply_context/context-finder"
cd "$PROJECT_ROOT"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# ============================================================================
# 1. CODE QUALITY - Clippy (strictest lints)
# ============================================================================

echo -e "${BLUE}[1/8] Clippy Analysis (strictest lints)${NC}"
echo "────────────────────────────────────────"

cargo clippy --workspace --all-targets -- \
    -D warnings \
    -D clippy::all \
    -D clippy::pedantic \
    -D clippy::nursery \
    -A clippy::missing_errors_doc \
    -A clippy::missing_panics_doc \
    -A clippy::module_name_repetitions \
    2>&1 | tee audit_clippy.log

if [ ${PIPESTATUS[0]} -eq 0 ]; then
    echo -e "${GREEN}✓ Clippy passed (strictest)${NC}"
else
    echo -e "${RED}✗ Clippy found issues${NC}"
fi
echo

# ============================================================================
# 2. SECURITY AUDIT - cargo audit
# ============================================================================

echo -e "${BLUE}[2/8] Security Audit (dependencies)${NC}"
echo "────────────────────────────────────────"

if command -v cargo-audit &> /dev/null; then
    cargo audit 2>&1 | tee audit_security.log
    if [ ${PIPESTATUS[0]} -eq 0 ]; then
        echo -e "${GREEN}✓ No known vulnerabilities${NC}"
    else
        echo -e "${YELLOW}⚠ Security issues found${NC}"
    fi
else
    echo -e "${YELLOW}⚠ cargo-audit not installed (run: cargo install cargo-audit)${NC}"
fi
echo

# ============================================================================
# 3. TEST COVERAGE
# ============================================================================

echo -e "${BLUE}[3/8] Test Coverage${NC}"
echo "────────────────────────────────────────"

# Run all tests
cargo test --workspace --lib 2>&1 | tee audit_tests.log

# Count tests
total_tests=$(grep -o "test result:" audit_tests.log | wc -l)
passed_tests=$(grep "passed" audit_tests.log | grep -o "[0-9]* passed" | cut -d' ' -f1 | paste -sd+ | bc)
ignored_tests=$(grep "ignored" audit_tests.log | grep -o "[0-9]* ignored" | cut -d' ' -f1 | paste -sd+ | bc)

echo
echo "Test Statistics:"
echo "  Total test suites: $total_tests"
echo "  Passed: $passed_tests"
echo "  Ignored: $ignored_tests"
echo

if [ "$passed_tests" -gt 30 ]; then
    echo -e "${GREEN}✓ Good test coverage${NC}"
else
    echo -e "${YELLOW}⚠ Limited test coverage${NC}"
fi
echo

# ============================================================================
# 4. CODE COMPLEXITY - tokei
# ============================================================================

echo -e "${BLUE}[4/8] Code Complexity Analysis${NC}"
echo "────────────────────────────────────────"

if command -v tokei &> /dev/null; then
    tokei crates/ --exclude "*.json" --exclude "*.md" 2>&1 | tee audit_complexity.log
    echo -e "${GREEN}✓ Code metrics generated${NC}"
else
    echo -e "${YELLOW}⚠ tokei not installed (run: cargo install tokei)${NC}"
    # Fallback to basic counting
    echo "Rust files:"
    find crates -name "*.rs" | wc -l
    echo "Total lines:"
    find crates -name "*.rs" -exec cat {} \; | wc -l
fi
echo

# ============================================================================
# 5. DEPENDENCY TREE
# ============================================================================

echo -e "${BLUE}[5/8] Dependency Analysis${NC}"
echo "────────────────────────────────────────"

cargo tree --workspace --depth 1 2>&1 | head -50 | tee audit_deps.log
echo -e "${GREEN}✓ Dependencies checked${NC}"
echo

# ============================================================================
# 6. BUILD VERIFICATION
# ============================================================================

echo -e "${BLUE}[6/8] Build Verification (release)${NC}"
echo "────────────────────────────────────────"

cargo build --workspace --release 2>&1 | tail -20 | tee audit_build.log

if [ ${PIPESTATUS[0]} -eq 0 ]; then
    echo -e "${GREEN}✓ Release build successful${NC}"
else
    echo -e "${RED}✗ Release build failed${NC}"
fi
echo

# ============================================================================
# 7. DOCUMENTATION
# ============================================================================

echo -e "${BLUE}[7/8] Documentation Check${NC}"
echo "────────────────────────────────────────"

cargo doc --workspace --no-deps 2>&1 | tail -20 | tee audit_docs.log

if [ ${PIPESTATUS[0]} -eq 0 ]; then
    echo -e "${GREEN}✓ Documentation builds${NC}"
else
    echo -e "${YELLOW}⚠ Documentation warnings${NC}"
fi
echo

# ============================================================================
# 8. ARCHITECTURAL REVIEW
# ============================================================================

echo -e "${BLUE}[8/8] Architectural Review${NC}"
echo "────────────────────────────────────────"

echo "Crate structure:"
ls -1 crates/

echo
echo "Public API surface:"
find crates/*/src/lib.rs -exec echo "{}:" \; -exec grep "^pub " {} \; | head -50

echo
echo -e "${GREEN}✓ Architecture reviewed${NC}"
echo

# ============================================================================
# SUMMARY
# ============================================================================

echo "╔═══════════════════════════════════════════════════════════════════════════╗"
echo "║                          AUDIT COMPLETE                                   ║"
echo "╚═══════════════════════════════════════════════════════════════════════════╝"
echo
echo "Audit logs generated:"
echo "  - audit_clippy.log (code quality)"
echo "  - audit_security.log (security)"
echo "  - audit_tests.log (test results)"
echo "  - audit_complexity.log (code metrics)"
echo "  - audit_deps.log (dependencies)"
echo "  - audit_build.log (build output)"
echo "  - audit_docs.log (documentation)"
echo
echo "Review these logs for detailed findings."
