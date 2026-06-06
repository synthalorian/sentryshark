use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sentryshark::diff_filter::DiffFilter;
use sentryshark::inline_comments::ReviewParser;
use sentryshark::llm::LlmClient;
use sentryshark::config::ReviewConfig;

fn benchmark_diff_filter(c: &mut Criterion) {
    let diff = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {
-    let x = 1;
+    let x = 2;
 }
 diff --git a/Cargo.lock b/Cargo.lock
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1,5 +1,5 @@
 version = 1
-diff = old
+diff = new
"#;

    let filter = DiffFilter::new(
        &["Cargo.lock".to_string(), "*.lock".to_string()],
        &["dist/".to_string()],
        true,
    );

    c.bench_function("diff_filter", |b| {
        b.iter(|| filter.filter_diff(black_box(diff)))
    });
}

fn benchmark_review_parser(c: &mut Criterion) {
    let llm_output = r#"VERDICT: COMMENT
SUMMARY: Some issues found in the code review

FILE: src/main.rs
LINE: 42
COMMENT: This could panic, consider using unwrap_or_default instead

FILE: src/lib.rs
LINE: 10
COMMENT: Good documentation here
"#;

    c.bench_function("review_parser", |b| {
        b.iter(|| ReviewParser::parse(black_box(llm_output)))
    });
}

fn benchmark_prompt_building(c: &mut Criterion) {
    let config = ReviewConfig {
        security: true,
        style: true,
        performance: true,
        correctness: true,
        maintainability: true,
        inline_comments: true,
        summary_comment: true,
        template: None,
    };

    let client = LlmClient::new(
        "http://localhost:8080".to_string(),
        "test-model".to_string(),
        4096,
        0.1,
        config,
    );

    let diff = "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,5 +1,5 @@\n fn main() {\n-    let x = 1;\n+    let x = 2;\n }\n";

    c.bench_function("prompt_building", |b| {
        b.iter(|| client.build_prompt(black_box(diff)))
    });
}

criterion_group!(
    benches,
    benchmark_diff_filter,
    benchmark_review_parser,
    benchmark_prompt_building
);
criterion_main!(benches);
