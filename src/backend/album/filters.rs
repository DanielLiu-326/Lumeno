//! Album photo filters: typed kinds registered in a central registry, evaluated
//! left-to-right as a boolean chain.
//!
//! Wire format (JSON clause):
//! ```json
//! { "type": "rating", "logic": "and", "value": { "ratings": [4, 5] }, "enabled": true }
//! ```
//!
//! Each enabled predicate is combined with the result so far:
//!   first clause is the seed (`and_not` negates it; `and` / `or` are the predicate);
//!   later clauses apply `acc ∧ P`, `acc ∨ P`, or `acc ∧ ¬P`.
//! A semantic text clause with an empty query is an always-true predicate.
//! A non-empty query is skipped here and handled by the caller.

use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::backend::common::AppError;
use crate::backend::i18n::trf;

/// Something a photo filter can test against.
pub trait FilterablePhoto {
    fn path(&self) -> &str;
    fn name(&self) -> &str;
    fn date(&self) -> &str;
    fn rating(&self) -> u8;
    fn color_label_keys(&self) -> Vec<String>;
    fn camera_label(&self) -> String;
    fn lens(&self) -> &str;
    fn tag_list(&self) -> &[String];
    fn file_ext_lower(&self) -> String;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterLogic {
    And,
    Or,
    AndNot,
}

impl Default for FilterLogic {
    fn default() -> Self {
        Self::And
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterClause {
    #[serde(rename = "type")]
    pub filter_type: String,
    #[serde(default)]
    pub logic: FilterLogic,
    #[serde(default)]
    pub value: Value,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Extra data needed by some kinds (e.g. person → path allowlist).
#[derive(Debug, Default, Clone)]
pub struct FilterEvalCtx {
    /// When set, person filters match if photo.path is in this set.
    /// Built from all person-id clauses before evaluation.
    pub person_paths: Option<HashSet<String>>,
}

/// Compiled predicate for one clause.
pub trait PhotoPredicate: Send + Sync {
    fn matches(&self, photo: &dyn FilterablePhoto, ctx: &FilterEvalCtx) -> bool;
}

/// AI semantic text is not a per-photo index predicate; callers handle it.
#[derive(Debug, Clone)]
pub struct AiTextEffect {
    pub query: String,
    pub min_score: f32,
    pub logic: FilterLogic,
}

pub enum CompiledClause {
    Pred {
        logic: FilterLogic,
        pred: Arc<dyn PhotoPredicate>,
    },
    AiText(AiTextEffect),
}

/// One registered filter type.
pub trait FilterKind: Send + Sync {
    fn key(&self) -> &'static str;

    /// Compile JSON value into a predicate, or `None` if the value is empty.
    fn compile(&self, value: &Value) -> Result<Option<Arc<dyn PhotoPredicate>>, AppError>;

    /// Optional: compile into an AI text effect instead of a photo predicate.
    fn compile_ai_text(&self, _value: &Value) -> Result<Option<AiTextEffect>, AppError> {
        Ok(None)
    }

    fn is_ai_text(&self) -> bool {
        false
    }
}

pub struct FilterRegistry {
    kinds: HashMap<&'static str, Arc<dyn FilterKind>>,
}

impl FilterRegistry {
    pub fn new(kinds: Vec<Arc<dyn FilterKind>>) -> Self {
        let mut map = HashMap::new();
        for kind in kinds {
            map.insert(kind.key(), kind);
        }
        Self { kinds: map }
    }

    pub fn global() -> &'static FilterRegistry {
        use std::sync::OnceLock;
        static REGISTRY: OnceLock<FilterRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| FilterRegistry::new(all_kinds()))
    }

    pub fn compile_clauses(
        &self,
        clauses: &[FilterClause],
        locale: &str,
    ) -> Result<Vec<CompiledClause>, AppError> {
        let mut out = Vec::new();
        for clause in clauses {
            if !clause.enabled {
                continue;
            }
            let Some(kind) = self.kinds.get(clause.filter_type.as_str()) else {
                return Err(AppError::BadRequest(trf(
                    locale,
                    "error.bad_request",
                    &[(
                        "value",
                        format!("unknown filter type: {}", clause.filter_type),
                    )],
                )));
            };

            if kind.is_ai_text() {
                if let Some(mut effect) = kind.compile_ai_text(&clause.value)? {
                    effect.logic = clause.logic;
                    out.push(CompiledClause::AiText(effect));
                } else if let Some(pred) = kind.compile(&clause.value)? {
                    out.push(CompiledClause::Pred {
                        logic: clause.logic,
                        pred,
                    });
                }
                continue;
            }

            if let Some(pred) = kind.compile(&clause.value)? {
                out.push(CompiledClause::Pred {
                    logic: clause.logic,
                    pred,
                });
            }
        }
        Ok(out)
    }
}

/// Collect person ids referenced by person clauses (for building path sets).
pub fn collect_person_ids(clauses: &[FilterClause]) -> Vec<String> {
    let mut ids = Vec::new();
    for clause in clauses {
        if !clause.enabled || clause.filter_type != "person" {
            continue;
        }
        if let Some(arr) = clause.value.get("personIds").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(id) = item.as_str() {
                    let id = id.trim();
                    if !id.is_empty() {
                        ids.push(id.to_string());
                    }
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

pub fn extract_ai_text_effects(compiled: &[CompiledClause]) -> Vec<AiTextEffect> {
    compiled
        .iter()
        .filter_map(|c| match c {
            CompiledClause::AiText(effect) => Some(effect.clone()),
            _ => None,
        })
        .collect()
}

/// Left-to-right chain over compiled photo predicates.
/// Non-empty semantic text is skipped; an empty query is a normal predicate.
pub fn matches_compiled(
    photo: &dyn FilterablePhoto,
    compiled: &[CompiledClause],
    ctx: &FilterEvalCtx,
) -> bool {
    let mut acc: Option<bool> = None;

    for clause in compiled {
        let CompiledClause::Pred { logic, pred } = clause else {
            continue;
        };
        let matched = pred.matches(photo, ctx);
        acc = Some(match acc {
            None => match logic {
                FilterLogic::AndNot => !matched,
                FilterLogic::And | FilterLogic::Or => matched,
            },
            Some(prev) => match logic {
                FilterLogic::And => prev && matched,
                FilterLogic::Or => prev || matched,
                FilterLogic::AndNot => prev && !matched,
            },
        });
    }

    acc.unwrap_or(true)
}

/// Convert legacy flat query fields into a chain that matches the old bucket
/// semantics: `(all AND fields) ∧ (any OR camera/lens/format)`. OR clauses are
/// emitted first so later ANDs apply to the whole disjunction.
pub fn clauses_from_legacy_flat(
    from: Option<&str>,
    to: Option<&str>,
    tags: Option<&str>,
    ratings: Option<&str>,
    color_label: Option<&str>,
    cameras: Option<&str>,
    camera: Option<&str>,
    lenses: Option<&str>,
    lens: Option<&str>,
    formats: Option<&str>,
    format: Option<&str>,
    q: Option<&str>,
    person_ids: Option<&str>,
) -> Vec<FilterClause> {
    let mut ands = Vec::new();
    let mut ors = Vec::new();

    let from = from.unwrap_or("").trim();
    let to = to.unwrap_or("").trim();
    if !from.is_empty() || !to.is_empty() {
        ands.push(FilterClause {
            filter_type: "date_range".into(),
            logic: FilterLogic::And,
            value: serde_json::json!({ "from": from, "to": to }),
            enabled: true,
        });
    }

    let ratings: Vec<u8> = ratings
        .unwrap_or("")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if !ratings.is_empty() {
        ands.push(FilterClause {
            filter_type: "rating".into(),
            logic: FilterLogic::And,
            value: serde_json::json!({ "ratings": ratings }),
            enabled: true,
        });
    }

    let colors: Vec<String> = color_label
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if !colors.is_empty() {
        ands.push(FilterClause {
            filter_type: "color_label".into(),
            logic: FilterLogic::And,
            value: serde_json::json!({ "colorLabels": colors }),
            enabled: true,
        });
    }

    let mut cams: Vec<String> = cameras
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if cams.is_empty() {
        let single = camera.unwrap_or("").trim();
        if !single.is_empty() {
            cams.push(single.to_string());
        }
    }
    for cam in cams {
        ors.push(FilterClause {
            filter_type: "camera".into(),
            logic: FilterLogic::Or,
            value: serde_json::json!({ "camera": cam }),
            enabled: true,
        });
    }

    let mut lens_list: Vec<String> = lenses
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if lens_list.is_empty() {
        let single = lens.unwrap_or("").trim();
        if !single.is_empty() {
            lens_list.push(single.to_string());
        }
    }
    for item in lens_list {
        ors.push(FilterClause {
            filter_type: "lens".into(),
            logic: FilterLogic::Or,
            value: serde_json::json!({ "lens": item }),
            enabled: true,
        });
    }

    let mut fmt_list: Vec<String> = formats
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if fmt_list.is_empty() {
        let single = format.unwrap_or("").trim().to_lowercase();
        if !single.is_empty() {
            fmt_list.push(single);
        }
    }
    for item in fmt_list {
        ors.push(FilterClause {
            filter_type: "format".into(),
            logic: FilterLogic::Or,
            value: serde_json::json!({ "format": item }),
            enabled: true,
        });
    }

    let tags = tags.unwrap_or("").trim();
    if !tags.is_empty() {
        ands.push(FilterClause {
            filter_type: "tags".into(),
            logic: FilterLogic::And,
            value: serde_json::json!({ "tags": tags }),
            enabled: true,
        });
    }

    let q = q.unwrap_or("").trim();
    if !q.is_empty() {
        // Legacy `q` was filename/path/tags substring — map to metadata_text kind.
        ands.push(FilterClause {
            filter_type: "metadata_text".into(),
            logic: FilterLogic::And,
            value: serde_json::json!({ "q": q }),
            enabled: true,
        });
    }

    let persons: Vec<String> = person_ids
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if !persons.is_empty() {
        ands.push(FilterClause {
            filter_type: "person".into(),
            logic: FilterLogic::And,
            value: serde_json::json!({ "personIds": persons }),
            enabled: true,
        });
    }

    ors.extend(ands);
    ors
}

// ── Filter kinds ─────────────────────────────────────────────────────────────

fn empty_ok() -> Result<Option<Arc<dyn PhotoPredicate>>, AppError> {
    Ok(None)
}

// ── date_range ───────────────────────────────────────────────────────────────

struct DateRangeKind;
struct DateRangePred {
    from: String,
    to: String,
}

impl FilterKind for DateRangeKind {
    fn key(&self) -> &'static str {
        "date_range"
    }
    fn compile(&self, value: &Value) -> Result<Option<Arc<dyn PhotoPredicate>>, AppError> {
        let from = value
            .get("from")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let to = value
            .get("to")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if from.is_empty() && to.is_empty() {
            return empty_ok();
        }
        Ok(Some(Arc::new(DateRangePred { from, to })))
    }
}

impl PhotoPredicate for DateRangePred {
    fn matches(&self, photo: &dyn FilterablePhoto, _ctx: &FilterEvalCtx) -> bool {
        let date = photo.date();
        if !self.from.is_empty() && date < self.from.as_str() {
            return false;
        }
        if !self.to.is_empty() && date > self.to.as_str() {
            return false;
        }
        true
    }
}

// ── rating ───────────────────────────────────────────────────────────────────

struct RatingKind;
struct RatingPred {
    ratings: Vec<u8>,
}

impl FilterKind for RatingKind {
    fn key(&self) -> &'static str {
        "rating"
    }
    fn compile(&self, value: &Value) -> Result<Option<Arc<dyn PhotoPredicate>>, AppError> {
        let mut ratings = Vec::new();
        if let Some(arr) = value.get("ratings").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(n) = item.as_u64() {
                    ratings.push(n as u8);
                } else if let Some(s) = item.as_str() {
                    if let Ok(n) = s.parse::<u8>() {
                        ratings.push(n);
                    }
                }
            }
        }
        ratings.sort_unstable();
        ratings.dedup();
        if ratings.is_empty() {
            return empty_ok();
        }
        Ok(Some(Arc::new(RatingPred { ratings })))
    }
}

impl PhotoPredicate for RatingPred {
    fn matches(&self, photo: &dyn FilterablePhoto, _ctx: &FilterEvalCtx) -> bool {
        self.ratings.contains(&photo.rating())
    }
}

// ── color_label ──────────────────────────────────────────────────────────────

struct ColorLabelKind;
struct ColorLabelPred {
    labels: Vec<String>,
}

impl FilterKind for ColorLabelKind {
    fn key(&self) -> &'static str {
        "color_label"
    }
    fn compile(&self, value: &Value) -> Result<Option<Arc<dyn PhotoPredicate>>, AppError> {
        let mut labels = Vec::new();
        if let Some(arr) = value.get("colorLabels").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    let s = s.trim().to_lowercase();
                    if !s.is_empty() {
                        labels.push(s);
                    }
                }
            }
        }
        if labels.is_empty() {
            return empty_ok();
        }
        Ok(Some(Arc::new(ColorLabelPred { labels })))
    }
}

impl PhotoPredicate for ColorLabelPred {
    fn matches(&self, photo: &dyn FilterablePhoto, _ctx: &FilterEvalCtx) -> bool {
        let keys = photo.color_label_keys();
        keys.iter()
            .any(|k| self.labels.iter().any(|want| want == k))
    }
}

// ── format ───────────────────────────────────────────────────────────────────

struct FormatKind;
struct FormatPred {
    ext: String,
}

impl FilterKind for FormatKind {
    fn key(&self) -> &'static str {
        "format"
    }
    fn compile(&self, value: &Value) -> Result<Option<Arc<dyn PhotoPredicate>>, AppError> {
        let ext = value
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if ext.is_empty() {
            return empty_ok();
        }
        Ok(Some(Arc::new(FormatPred { ext })))
    }
}

impl PhotoPredicate for FormatPred {
    fn matches(&self, photo: &dyn FilterablePhoto, _ctx: &FilterEvalCtx) -> bool {
        photo.file_ext_lower() == self.ext
    }
}

// ── tags ─────────────────────────────────────────────────────────────────────

struct TagsKind;
struct TagsPred {
    tags: Vec<String>,
}

impl FilterKind for TagsKind {
    fn key(&self) -> &'static str {
        "tags"
    }
    fn compile(&self, value: &Value) -> Result<Option<Arc<dyn PhotoPredicate>>, AppError> {
        let raw = value
            .get("tags")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tags: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if tags.is_empty() {
            return empty_ok();
        }
        Ok(Some(Arc::new(TagsPred { tags })))
    }
}

impl PhotoPredicate for TagsPred {
    fn matches(&self, photo: &dyn FilterablePhoto, _ctx: &FilterEvalCtx) -> bool {
        let photo_tags: Vec<String> = photo.tag_list().iter().map(|t| t.to_lowercase()).collect();
        self.tags.iter().all(|t| photo_tags.iter().any(|p| p == t))
    }
}

// ── camera ───────────────────────────────────────────────────────────────────

struct CameraKind;
struct CameraPred {
    camera: String,
}

impl FilterKind for CameraKind {
    fn key(&self) -> &'static str {
        "camera"
    }
    fn compile(&self, value: &Value) -> Result<Option<Arc<dyn PhotoPredicate>>, AppError> {
        let camera = value
            .get("camera")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if camera.is_empty() {
            return empty_ok();
        }
        Ok(Some(Arc::new(CameraPred { camera })))
    }
}

impl PhotoPredicate for CameraPred {
    fn matches(&self, photo: &dyn FilterablePhoto, _ctx: &FilterEvalCtx) -> bool {
        photo.camera_label().to_lowercase() == self.camera
    }
}

// ── lens ─────────────────────────────────────────────────────────────────────

struct LensKind;
struct LensPred {
    lens: String,
}

impl FilterKind for LensKind {
    fn key(&self) -> &'static str {
        "lens"
    }
    fn compile(&self, value: &Value) -> Result<Option<Arc<dyn PhotoPredicate>>, AppError> {
        let lens = value
            .get("lens")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if lens.is_empty() {
            return empty_ok();
        }
        Ok(Some(Arc::new(LensPred { lens })))
    }
}

impl PhotoPredicate for LensPred {
    fn matches(&self, photo: &dyn FilterablePhoto, _ctx: &FilterEvalCtx) -> bool {
        photo.lens().to_lowercase() == self.lens
    }
}

// ── metadata_text (filename / path / tags substring) ─────────────────────────

struct MetadataTextKind;
struct MetadataTextPred {
    needle: String,
}

impl FilterKind for MetadataTextKind {
    fn key(&self) -> &'static str {
        "metadata_text"
    }
    fn compile(&self, value: &Value) -> Result<Option<Arc<dyn PhotoPredicate>>, AppError> {
        let needle = value
            .get("q")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if needle.is_empty() {
            return empty_ok();
        }
        Ok(Some(Arc::new(MetadataTextPred { needle })))
    }
}

impl PhotoPredicate for MetadataTextPred {
    fn matches(&self, photo: &dyn FilterablePhoto, _ctx: &FilterEvalCtx) -> bool {
        let name = photo.name().to_lowercase();
        let path = photo.path().to_lowercase();
        let tags = photo.tag_list().join(" ").to_lowercase();
        name.contains(&self.needle) || path.contains(&self.needle) || tags.contains(&self.needle)
    }
}

// ── person ───────────────────────────────────────────────────────────────────

struct PersonKind;
struct PersonPred;

impl FilterKind for PersonKind {
    fn key(&self) -> &'static str {
        "person"
    }
    fn compile(&self, value: &Value) -> Result<Option<Arc<dyn PhotoPredicate>>, AppError> {
        let empty = value
            .get("personIds")
            .and_then(|v| v.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(true);
        if empty {
            return empty_ok();
        }
        // Actual path matching uses FilterEvalCtx.person_paths built from all person ids.
        Ok(Some(Arc::new(PersonPred)))
    }
}

impl PhotoPredicate for PersonPred {
    fn matches(&self, photo: &dyn FilterablePhoto, ctx: &FilterEvalCtx) -> bool {
        match &ctx.person_paths {
            Some(paths) => paths.contains(photo.path()),
            None => true,
        }
    }
}

// ── text (empty query is always true; a query is handled by the caller) ─────

struct AiTextKind;

/// Blank semantic text matches every photo and never loads the text model.
struct AlwaysTrue;

impl PhotoPredicate for AlwaysTrue {
    fn matches(&self, _: &dyn FilterablePhoto, _: &FilterEvalCtx) -> bool {
        true
    }
}

fn ai_text_query(value: &Value) -> String {
    value
        .get("q")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

impl FilterKind for AiTextKind {
    fn key(&self) -> &'static str {
        "text"
    }
    fn is_ai_text(&self) -> bool {
        true
    }
    fn compile(&self, value: &Value) -> Result<Option<Arc<dyn PhotoPredicate>>, AppError> {
        if ai_text_query(value).is_empty() {
            return Ok(Some(Arc::new(AlwaysTrue)));
        }
        empty_ok()
    }
    fn compile_ai_text(&self, value: &Value) -> Result<Option<AiTextEffect>, AppError> {
        let query = ai_text_query(value);
        if query.is_empty() {
            return Ok(None);
        }
        let min_score = value
            .get("minScore")
            .and_then(|v| v.as_f64())
            .or_else(|| value.get("min_score").and_then(|v| v.as_f64()))
            .unwrap_or(0.0) as f32;
        Ok(Some(AiTextEffect {
            query,
            min_score: min_score.clamp(0.0, 1.0),
            logic: FilterLogic::And,
        }))
    }
}

/// Ordered registry of all filter kinds.
pub static FILTER_REGISTRY: &[fn() -> Arc<dyn FilterKind>] = &[
    || Arc::new(DateRangeKind),
    || Arc::new(RatingKind),
    || Arc::new(ColorLabelKind),
    || Arc::new(FormatKind),
    || Arc::new(TagsKind),
    || Arc::new(CameraKind),
    || Arc::new(LensKind),
    || Arc::new(MetadataTextKind),
    || Arc::new(PersonKind),
    || Arc::new(AiTextKind),
];

// Re-export as constructed arcs for FilterRegistry::new
pub fn all_kinds() -> Vec<Arc<dyn FilterKind>> {
    FILTER_REGISTRY.iter().map(|f| f()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyPhoto;

    impl FilterablePhoto for DummyPhoto {
        fn path(&self) -> &str {
            ""
        }
        fn name(&self) -> &str {
            ""
        }
        fn date(&self) -> &str {
            ""
        }
        fn rating(&self) -> u8 {
            0
        }
        fn color_label_keys(&self) -> Vec<String> {
            vec![]
        }
        fn camera_label(&self) -> String {
            String::new()
        }
        fn lens(&self) -> &str {
            ""
        }
        fn tag_list(&self) -> &[String] {
            static EMPTY: &[String] = &[];
            EMPTY
        }
        fn file_ext_lower(&self) -> String {
            String::new()
        }
    }

    struct FlagPred(bool);

    impl PhotoPredicate for FlagPred {
        fn matches(&self, _: &dyn FilterablePhoto, _: &FilterEvalCtx) -> bool {
            self.0
        }
    }

    fn pred(logic: FilterLogic, flag: bool) -> CompiledClause {
        CompiledClause::Pred {
            logic,
            pred: Arc::new(FlagPred(flag)),
        }
    }

    fn eval(clauses: &[CompiledClause]) -> bool {
        matches_compiled(&DummyPhoto, clauses, &FilterEvalCtx::default())
    }

    #[test]
    fn empty_chain_matches() {
        assert!(eval(&[]));
    }

    #[test]
    fn first_and_or_are_the_predicate() {
        assert!(eval(&[pred(FilterLogic::And, true)]));
        assert!(!eval(&[pred(FilterLogic::And, false)]));
        assert!(eval(&[pred(FilterLogic::Or, true)]));
        assert!(!eval(&[pred(FilterLogic::Or, false)]));
    }

    #[test]
    fn first_and_not_negates() {
        assert!(!eval(&[pred(FilterLogic::AndNot, true)]));
        assert!(eval(&[pred(FilterLogic::AndNot, false)]));
    }

    #[test]
    fn and_then_or_is_left_associative() {
        // (T ∧ F) ∨ T = T  (old buckets: T ∧ F ∧ T = F)
        assert!(eval(&[
            pred(FilterLogic::And, true),
            pred(FilterLogic::And, false),
            pred(FilterLogic::Or, true),
        ]));
        // (T ∧ F) ∨ F = F
        assert!(!eval(&[
            pred(FilterLogic::And, true),
            pred(FilterLogic::And, false),
            pred(FilterLogic::Or, false),
        ]));
    }

    #[test]
    fn or_then_and_distributes() {
        // (T ∨ F) ∧ F = F
        assert!(!eval(&[
            pred(FilterLogic::And, true),
            pred(FilterLogic::Or, false),
            pred(FilterLogic::And, false),
        ]));
        // (F ∨ T) ∧ T = T
        assert!(eval(&[
            pred(FilterLogic::And, false),
            pred(FilterLogic::Or, true),
            pred(FilterLogic::And, true),
        ]));
    }

    #[test]
    fn and_not_in_chain() {
        assert!(eval(&[
            pred(FilterLogic::And, true),
            pred(FilterLogic::AndNot, false),
        ]));
        assert!(!eval(&[
            pred(FilterLogic::And, true),
            pred(FilterLogic::AndNot, true),
        ]));
    }

    #[test]
    fn skips_ai_text_in_the_chain() {
        let clauses = vec![
            pred(FilterLogic::And, true),
            CompiledClause::AiText(AiTextEffect {
                query: "x".into(),
                min_score: 0.0,
                logic: FilterLogic::And,
            }),
            pred(FilterLogic::Or, false),
        ];
        // T ∨ F (AI skipped)
        assert!(eval(&clauses));
    }

    #[test]
    fn legacy_flat_ors_then_ands() {
        let clauses = clauses_from_legacy_flat(
            Some("2020-01-01"),
            None,
            Some("beach"),
            None,
            None,
            Some("CamA,CamB"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let kinds: Vec<_> = clauses
            .iter()
            .map(|c| (c.filter_type.as_str(), c.logic))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("camera", FilterLogic::Or),
                ("camera", FilterLogic::Or),
                ("date_range", FilterLogic::And),
                ("tags", FilterLogic::And),
            ]
        );
    }

    #[test]
    fn empty_ai_text_is_true_and_does_not_become_an_effect() {
        let registry = FilterRegistry::global();
        let empty: Vec<FilterClause> = serde_json::from_value(serde_json::json!([
            { "type": "rating", "logic": "and", "enabled": true, "value": { "ratings": [5] } },
            { "type": "text", "logic": "or", "enabled": true, "value": { "q": "  " } }
        ]))
        .unwrap();
        let compiled = registry.compile_clauses(&empty, "en").unwrap();
        assert!(extract_ai_text_effects(&compiled).is_empty());
        // Rating 5 misses DummyPhoto; blank text is true, so false ∨ true.
        assert!(eval(&compiled));

        let negated: Vec<FilterClause> = serde_json::from_value(serde_json::json!([
            { "type": "text", "logic": "and_not", "enabled": true, "value": { "q": "" } }
        ]))
        .unwrap();
        let compiled = registry.compile_clauses(&negated, "en").unwrap();
        assert!(!eval(&compiled));

        let query: Vec<FilterClause> = serde_json::from_value(serde_json::json!([
            { "type": "text", "logic": "and", "enabled": true, "value": { "q": "海边" } }
        ]))
        .unwrap();
        let compiled = registry.compile_clauses(&query, "en").unwrap();
        let effects = extract_ai_text_effects(&compiled);
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].query, "海边");
        assert!(matches!(compiled[0], CompiledClause::AiText(_)));
    }
}
