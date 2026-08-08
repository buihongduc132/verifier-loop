# Schema Versioning for Hash-Chained, Signature-Bound JSON Records

> Date: 2026-07-08
> Status: research (input for implementation task)
> Scope: Add `schemaVersion` to `VerdictRecord`, `CompletionRecord`, `ReceiptEntry` without breaking hash chains, signatures, or backward compatibility.

## 1. Current State

Three on-disk record types are serialized as JSON:

| Record | File | Signed? | Hash-chained? |
|--------|------|---------|---------------|
| `VerdictRecord` | `verdict.json` | Yes (Ed25519 over `canonical_record_bytes`) | No (but feeds into receipt log + completion hash) |
| `CompletionRecord` | `completion.json` | No | No (but its `fullDigest` binds specific fields via SHA-256) |
| `ReceiptEntry` | `receipt-log.jsonl` | No | Yes (`entryHash = SHA256(prevHash\|seq\|kind\|verdictId\|status)`) |

**Critical observation**: All three cryptographic bindings (signature canonical bytes, completion hash, receipt entry hash) are constructed **manually** via explicit field selection — NOT by serializing the struct. This means adding a new field to the struct does NOT automatically change the cryptographic inputs.

### Existing version-tolerance pattern

`VerdictRecord` already uses `#[serde(default, skip_serializing_if = "Option::is_none")]` for `notes`, `registeredAt`, `signature`, `pubkeyId`. This is the established pattern for forward-compatible fields.

### Existing design precedent (R5)

The tamper-hardening design already addresses this:

> **[R5]** Canonical-JSON signature byte-reproducibility across versions → D7 fixes the field set; a future field addition changes the canonical bytes and breaks old sigs (intended — signatures are per-record-version). Auditor code must pin the field set per record version.

This confirms: **schemaVersion must NOT be added to the signature canonical bytes**. It is advisory metadata.

---

## 2. Research Questions & Answers

### Q1: `#[serde(default)]` vs `Option<T>` — which is more robust?

| Approach | Missing field behavior | Serialization | Type safety |
|----------|----------------------|---------------|-------------|
| `#[serde(default)]` on `T` | Uses `T::default()` | Always present in output | Caller can't distinguish "absent" from "default" |
| `#[serde(default)]` on `Option<T>` | `None` | Skippable via `skip_serializing_if` | Caller explicitly handles absent vs present |
| Bare `Option<T>` (no default) | **Deserialization error** | — | N/A — broken |

**Recommendation**: `Option<u32>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`. This is:
- Consistent with the existing pattern in `VerdictRecord`.
- Self-documenting: `None` means "written by pre-versioning code".
- Round-trips correctly: old records deserialize to `None`, new records serialize with `Some(1)`.

### Q2: Versioning strategy — which is simplest for ~3 record types, 2 schema versions?

| Strategy | Complexity | Fit for this project |
|----------|-----------|---------------------|
| **Integer `schemaVersion`** | Minimal | ✅ Best fit. One field, one number. |
| Semver string `"1.0.0"` | Overkill | ❌ No API contract to version. |
| Feature flags / capability bits | Complex | ❌ No conditional logic needed yet. |
| Per-record version fields | Redundant | ❌ All records evolve together in this project. |
| No version (rely on field presence) | Implicit | ⚠️ Works but fragile — hard to audit "which version wrote this?" |

**Recommendation**: Plain integer `schemaVersion: Option<u32>`. Value `1` = current schema. Absent (`None`) = pre-versioning (implicitly version 0).

### Q3: How do other hash-chained systems handle schema evolution?

| System | Approach | Key insight |
|--------|----------|-------------|
| **Bitcoin** | `nVersion` field in blocks/txs. Advisory — not part of the txid hash. Old txs still valid. | Version is metadata, not a cryptographic input. |
| **Git** | No version field. Objects are content-addressed by SHA. Schema changes = new object type. | Impractical for append-only logs. |
| **Certificate Transparency** | Version in TLS extension struct. Old entries remain valid. | Version field is outside the signed TBSCertificate. |
| **Protocol Buffers** | Field numbers + `reserved`. Unknown fields are preserved on round-trip. | Wire format handles evolution natively. |
| **JSON-LD / Schema.org** | `@context` URL acts as schema identifier. | Overkill for fixed-schema records. |

**Common pattern**: The version field is **advisory** — it sits alongside the cryptographic bindings but is NOT included in them. This allows old records to remain valid while new records carry version metadata.

### Q4: Should `schemaVersion` be part of the signature canonical bytes?

**NO.** Here's the decision tree:

```
Include schemaVersion in canonical bytes?
├── YES → Old records (without schemaVersion) produce different canonical bytes
│         → Old signatures fail verification
│         → BREAKING: all existing signed verdicts become unverifiable
│         → Also breaks completion hash (receipt_head changes)
│
└── NO  → Old records verify normally (canonical bytes unchanged)
          → New records also verify (schemaVersion is just extra JSON field)
          → schemaVersion is advisory: tells the reader which schema was used
          → ✅ Non-breaking
```

**Same reasoning applies to**:
- Completion hash formula: `schemaVersion` must NOT be an input to `compute_hash()`.
- Receipt entry hash: `schemaVersion` must NOT be in `compute_entry_hash()`.

### Q5: Minimal change to add `schemaVersion=1`

The change is additive and non-breaking:

1. Add `schema_version: Option<u32>` to all three structs with serde annotations.
2. Set `Some(1)` when constructing new records.
3. Leave `None` for legacy records (deserialization handles this automatically).
4. Do NOT modify `canonical_record_bytes()`, `compute_hash()`, or `compute_entry_hash()`.

---

## 3. Recommended Approach

### Pattern: Advisory version field with serde `Option`

```rust
// ── VerdictRecord ──────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerdictRecord {
    pub status: VerdictStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubkey_id: Option<String>,
    /// Schema version of this record. `None` = pre-versioning (v0).
    /// `Some(1)` = current schema. NOT part of the signature canonical bytes
    /// or completion hash — advisory only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
}

// ── CompletionRecord ───────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionRecord {
    pub hash: String,
    pub full_digest: String,
    pub goal_id: String,
    pub round_number: u32,
    pub matched_at: String,
    pub matching_verdicts: Vec<MatchingVerdict>,
    /// Schema version. `None` = pre-versioning (v0). `Some(1)` = current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
}

// ── ReceiptEntry ───────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptEntry {
    pub seq: u64,
    pub kind: String,
    pub verdict_id: String,
    pub status: String,
    pub prev_hash: String,
    pub entry_hash: String,
    pub signed_by: String,
    /// Schema version. `None` = pre-versioning (v0). `Some(1)` = current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
}
```

### Construction sites (set `Some(1)` on write)

| Location | Record | Change |
|----------|--------|--------|
| `verdict::build_signed_record()` | `VerdictRecord` | Add `schema_version: Some(1)` |
| `verdict::register_approve()` | `VerdictRecord` | Add `schema_version: Some(1)` |
| `verdict::register_reject()` | `VerdictRecord` | Add `schema_version: Some(1)` |
| `verdict::read_verdict()` null path | `VerdictRecord` | Add `schema_version: None` |
| `consensus::write_completion()` | `CompletionRecord` | Add `schema_version: Some(1)` |
| `receipt::append_receipt_locked()` | `ReceiptEntry` | Add `schema_version: Some(1)` |

### What does NOT change

- `crypto::canonical_record_bytes()` — field set is pinned (R5).
- `consensus::compute_hash()` — hash formula is pinned.
- `receipt::compute_entry_hash()` — chain formula is pinned.
- All existing tests — old JSON without `schemaVersion` deserializes to `None`.

---

## 4. Trade-offs

| Pro | Con |
|-----|-----|
| Non-breaking: old records read as `None` | `schemaVersion` is advisory only — a malicious writer can set any value |
| Consistent with existing `Option` pattern in `VerdictRecord` | Doesn't prevent schema drift between writer and reader code versions |
| Minimal code change (~6 struct fields + ~6 construction sites) | Future schema changes still need manual coordination |
| Self-documenting: `None` vs `Some(1)` is unambiguous | Two records with identical content but different `schemaVersion` have different JSON but same cryptographic bindings |
| Auditor can detect which version wrote a record | — |

---

## 5. Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| Future developer adds `schemaVersion` to canonical bytes | HIGH — breaks all existing signatures | Document in code comment + AGENTS.md that version fields are excluded from crypto bindings |
| `schemaVersion` accidentally included in hash formula | HIGH — breaks existing completion hashes | Same documentation; also, existing tests pin the hash formula |
| Round-trip fidelity: read old → write back adds `schemaVersion` | LOW — acceptable behavior (upgrading on write) | Document as intentional: re-serializing a v0 record produces v1 |
| Third-party tools parse the JSON and choke on new field | LOW — JSON is inherently extensible | `schemaVersion` is a top-level field; no nested structure changes |

---

## 6. Future Evolution (v2+)

When a breaking schema change is needed (e.g., new required field in the canonical bytes):

1. Bump `schemaVersion` to `2`.
2. Update `canonical_record_bytes()` to include the new field.
3. Old v1 signatures remain valid for v1 records (the canonical bytes for v1 records don't change).
4. New v2 records get v2 canonical bytes and v2 signatures.
5. Verification code branches on `schema_version`:
   ```rust
   let canonical = match record.schema_version.unwrap_or(0) {
       0 | 1 => crypto::canonical_record_bytes_v1(...),
       2 => crypto::canonical_record_bytes_v2(...),
       _ => return Err(VerdictError::UnsupportedSchema),
   };
   ```

This is the "version-pinned canonical form" pattern from R5.

---

## 7. References

- R5 in `openspec/changes/add-verifier-tamper-hardening/design.md`: "a future field addition changes the canonical bytes and breaks old sigs (intended — signatures are per-record-version)"
- D7: canonical serialization uses `BTreeMap` with fixed field set
- Existing pattern: `VerdictRecord` already uses `#[serde(default, skip_serializing_if = "Option::is_none")]` for 4 optional fields
- Bitcoin block version field: advisory, not part of block hash
- Certificate Transparency: version in extension struct, outside signed TBSCertificate
