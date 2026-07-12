use super::*;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

const TURN_EVIDENCE_OBJECT_KIND: &str = "TurnEvidenceManifest";
const SESSION_ATTESTATION_OBJECT_KIND: &str = "SessionAttestation";

impl Trail {
    /// Freeze the exact evidence already attached to one completed turn.
    pub fn create_turn_evidence_manifest(&mut self, turn_id: &str) -> Result<TurnEvidenceManifest> {
        if let Some(existing) = self.try_turn_evidence_manifest(turn_id)? {
            return Ok(existing);
        }
        let turn = self.lane_turn(turn_id)?;
        if turn.ended_at.is_none() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` must be completed before its evidence manifest is frozen"
            )));
        }
        let session_id = turn.session_id.clone().ok_or_else(|| {
            Error::InvalidInput(format!("turn `{turn_id}` has no owning session"))
        })?;
        let events = self.lane_turn_events(turn_id)?;
        let messages = self.lane_turn_messages(turn_id)?;
        let artifacts = self.list_lane_artifacts(&session_id, Some(turn_id), 1_000)?;
        let mut coverage = TurnEvidenceCoverage {
            event_ids: events
                .iter()
                .map(|event| {
                    event
                        .payload
                        .as_ref()
                        .and_then(|payload| payload.get("event_id"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(&event.event_id)
                        .to_string()
                })
                .collect(),
            message_ids: messages
                .iter()
                .map(|message| message.id.0.clone())
                .collect(),
            artifact_ids: artifacts
                .iter()
                .map(|artifact| artifact.artifact_id.clone())
                .collect(),
            ..TurnEvidenceCoverage::default()
        };
        for event in &events {
            if let Some(payload) = event.payload.as_ref() {
                collect_json_strings(payload, "receipt_id", &mut coverage.receipt_ids);
                collect_json_strings(payload, "span_id", &mut coverage.tool_span_ids);
                collect_json_strings(payload, "approval_id", &mut coverage.approval_ids);
            }
        }
        let after_change = turn
            .after_change
            .clone()
            .unwrap_or_else(|| turn.before_change.clone());
        if after_change != turn.before_change {
            coverage.change_ids.push(after_change.clone());
        }
        sort_dedup_coverage(&mut coverage);
        let statement = TurnEvidenceStatement {
            schema: TURN_EVIDENCE_MANIFEST_SCHEMA.to_string(),
            version: TURN_EVIDENCE_MANIFEST_VERSION,
            workspace_id: self.config.workspace.id.0.clone(),
            lane_id: turn.lane_id.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.to_string(),
            before_change: turn.before_change,
            after_change,
            turn_status: turn.status,
            coverage,
        };
        let statement_bytes = canonical_json_bytes(&statement)?;
        let digest = digest_bytes(&statement_bytes);
        let manifest_id = format!("evidence_{}", crate::ids::short_hash(digest.as_bytes(), 24));
        let object_id = self.put_object(
            TURN_EVIDENCE_OBJECT_KIND,
            TURN_EVIDENCE_MANIFEST_VERSION,
            &statement,
        )?;
        let created_at = now_millis();
        {
            let _lock = self.acquire_write_lock()?;
            self.conn.execute(
                "INSERT INTO lane_turn_evidence_manifests
                 (manifest_id, lane_id, session_id, turn_id, schema_version,
                  object_id, digest, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(turn_id) DO NOTHING",
                params![
                    manifest_id,
                    statement.lane_id,
                    session_id,
                    turn_id,
                    TURN_EVIDENCE_MANIFEST_VERSION,
                    object_id.0,
                    digest,
                    created_at,
                ],
            )?;
        }
        let stored = self.turn_evidence_manifest(turn_id)?;
        if stored.digest != digest {
            return Err(Error::Conflict(format!(
                "turn `{turn_id}` already has a different immutable evidence manifest"
            )));
        }
        Ok(stored)
    }

    pub fn turn_evidence_manifest(&self, turn_id: &str) -> Result<TurnEvidenceManifest> {
        self.try_turn_evidence_manifest(turn_id)?
            .ok_or_else(|| Error::ObjectNotFound {
                kind: "turn evidence manifest",
                id: turn_id.to_string(),
            })
    }

    pub fn list_turn_evidence_manifests(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<TurnEvidenceManifest>> {
        self.lane_session(session_id)?;
        let mut statement = self.conn.prepare(
            "SELECT manifest_id, lane_id, session_id, turn_id, schema_version,
                    object_id, digest, created_at
             FROM lane_turn_evidence_manifests
             WHERE session_id = ?1
             ORDER BY created_at, turn_id LIMIT ?2",
        )?;
        let rows = statement
            .query_map(
                params![
                    session_id,
                    i64::try_from(limit.clamp(1, 1_000)).unwrap_or(1_000)
                ],
                evidence_manifest_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|row| self.materialize_turn_evidence_manifest(row))
            .collect()
    }

    fn try_turn_evidence_manifest(&self, turn_id: &str) -> Result<Option<TurnEvidenceManifest>> {
        let row = self
            .conn
            .query_row(
                "SELECT manifest_id, lane_id, session_id, turn_id, schema_version,
                        object_id, digest, created_at
                 FROM lane_turn_evidence_manifests WHERE turn_id = ?1",
                params![turn_id],
                evidence_manifest_row,
            )
            .optional()?;
        row.map(|row| self.materialize_turn_evidence_manifest(row))
            .transpose()
    }

    fn materialize_turn_evidence_manifest(
        &self,
        row: EvidenceManifestRow,
    ) -> Result<TurnEvidenceManifest> {
        let statement: TurnEvidenceStatement =
            self.get_object(TURN_EVIDENCE_OBJECT_KIND, &row.object_id)?;
        if statement.schema != TURN_EVIDENCE_MANIFEST_SCHEMA
            || statement.version != row.schema_version
            || statement.turn_id != row.turn_id
            || statement.session_id != row.session_id
            || statement.lane_id != row.lane_id
        {
            return Err(Error::Corrupt(format!(
                "turn evidence manifest `{}` row/object identity mismatch",
                row.manifest_id
            )));
        }
        let actual_digest = digest_bytes(&canonical_json_bytes(&statement)?);
        if actual_digest != row.digest {
            return Err(Error::Corrupt(format!(
                "turn evidence manifest `{}` digest mismatch",
                row.manifest_id
            )));
        }
        Ok(TurnEvidenceManifest {
            manifest_id: row.manifest_id,
            lane_id: row.lane_id,
            session_id: row.session_id,
            turn_id: row.turn_id,
            schema_version: row.schema_version,
            object_id: row.object_id,
            digest: row.digest,
            created_at: row.created_at,
            statement,
        })
    }

    pub fn create_provenance_node(&mut self, input: ProvenanceNodeInput) -> Result<ProvenanceNode> {
        validate_evidence_text("provenance node kind", &input.node_kind, 128)?;
        validate_evidence_text("provenance summary", &input.summary, 4_096)?;
        validate_evidence_text(
            "provenance source confidence",
            &input.source_confidence,
            128,
        )?;
        let session = self.lane_session(&input.session_id)?;
        validate_optional_turn_for_session(self, input.turn_id.as_deref(), &input.session_id)?;
        if let Some(artifact_id) = input.artifact_id.as_deref() {
            let artifact = self.lane_artifact(artifact_id)?;
            if artifact.session_id != input.session_id {
                return Err(Error::InvalidInput(format!(
                    "artifact `{artifact_id}` does not belong to session `{}`",
                    input.session_id
                )));
            }
        }
        if let Some(message_id) = input.message_id.as_deref() {
            let message = self.message(message_id)?;
            if message.session_id.as_deref() != Some(input.session_id.as_str()) {
                return Err(Error::InvalidInput(format!(
                    "message `{message_id}` does not belong to session `{}`",
                    input.session_id
                )));
            }
        }
        let attributes_json = input
            .attributes
            .as_ref()
            .map(canonical_json_string)
            .transpose()?;
        let identity = canonical_json_bytes(&serde_json::json!({
            "session_id": input.session_id,
            "turn_id": input.turn_id,
            "node_kind": input.node_kind,
            "event_id": input.event_id,
            "span_id": input.span_id,
            "message_id": input.message_id,
            "change_id": input.change_id,
            "artifact_id": input.artifact_id,
            "source_confidence": input.source_confidence,
            "classifier_version": input.classifier_version,
            "attributes": input.attributes,
        }))?;
        let node_id = format!("provenance_node_{}", crate::ids::short_hash(&identity, 24));
        let now = now_millis();
        let _lock = self.acquire_write_lock()?;
        self.conn.execute(
            "INSERT INTO lane_provenance_nodes
             (provenance_node_id, lane_id, session_id, turn_id, node_kind, summary,
              event_id, span_id, message_id, change_id, artifact_id, source_confidence,
              classifier_version, created_at, attributes_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(provenance_node_id) DO NOTHING",
            params![
                node_id,
                session.lane_id,
                input.session_id,
                input.turn_id,
                input.node_kind,
                redact_sensitive_text(&input.summary),
                input.event_id,
                input.span_id,
                input.message_id,
                input.change_id,
                input.artifact_id,
                input.source_confidence,
                input.classifier_version,
                now,
                attributes_json,
            ],
        )?;
        self.provenance_node(&node_id)
    }

    pub fn provenance_node(&self, node_id: &str) -> Result<ProvenanceNode> {
        self.conn
            .query_row(
                PROVENANCE_NODE_SELECT_BY_ID,
                params![node_id],
                provenance_node_row,
            )
            .optional()?
            .ok_or_else(|| Error::ObjectNotFound {
                kind: "provenance node",
                id: node_id.to_string(),
            })
    }

    pub fn create_provenance_edge(&mut self, input: ProvenanceEdgeInput) -> Result<ProvenanceEdge> {
        validate_evidence_text("provenance relation", &input.relation, 128)?;
        validate_evidence_text(
            "provenance source confidence",
            &input.source_confidence,
            128,
        )?;
        let from = self.provenance_node(&input.from_node_id)?;
        let to = self.provenance_node(&input.to_node_id)?;
        if from.session_id != to.session_id || from.lane_id != to.lane_id {
            return Err(Error::InvalidInput(
                "provenance edges cannot cross session or lane boundaries".to_string(),
            ));
        }
        if let Some(receipt_id) = input.receipt_id.as_deref() {
            let receipt = self.agent_hook_receipt(receipt_id)?;
            if receipt.mapping_id.is_none() {
                return Err(Error::InvalidInput(format!(
                    "receipt `{receipt_id}` is not mapped to a captured session"
                )));
            }
        }
        let attributes_json = input
            .attributes
            .as_ref()
            .map(canonical_json_string)
            .transpose()?;
        let identity = format!(
            "{}:{}:{}:{}",
            input.from_node_id,
            input.to_node_id,
            input.relation,
            input.receipt_id.as_deref().unwrap_or("")
        );
        let edge_id = format!(
            "provenance_edge_{}",
            crate::ids::short_hash(identity.as_bytes(), 24)
        );
        let now = now_millis();
        let _lock = self.acquire_write_lock()?;
        self.conn.execute(
            "INSERT INTO lane_provenance_edges
             (provenance_edge_id, lane_id, session_id, from_node_id, to_node_id,
              relation, source_confidence, receipt_id, created_at, attributes_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(from_node_id, to_node_id, relation, COALESCE(receipt_id, ''))
             DO NOTHING",
            params![
                edge_id,
                from.lane_id,
                from.session_id,
                input.from_node_id,
                input.to_node_id,
                input.relation,
                input.source_confidence,
                input.receipt_id,
                now,
                attributes_json,
            ],
        )?;
        self.provenance_edge(&edge_id).or_else(|_| {
            self.conn
                .query_row(
                    "SELECT provenance_edge_id, lane_id, session_id, from_node_id,
                        to_node_id, relation, source_confidence, receipt_id,
                        created_at, attributes_json
                 FROM lane_provenance_edges
                 WHERE from_node_id = ?1 AND to_node_id = ?2 AND relation = ?3
                   AND COALESCE(receipt_id, '') = COALESCE(?4, '')",
                    params![
                        from.provenance_node_id,
                        to.provenance_node_id,
                        input.relation,
                        input.receipt_id
                    ],
                    provenance_edge_row,
                )
                .map_err(Error::from)
        })
    }

    pub fn provenance_edge(&self, edge_id: &str) -> Result<ProvenanceEdge> {
        self.conn
            .query_row(
                "SELECT provenance_edge_id, lane_id, session_id, from_node_id,
                        to_node_id, relation, source_confidence, receipt_id,
                        created_at, attributes_json
                 FROM lane_provenance_edges WHERE provenance_edge_id = ?1",
                params![edge_id],
                provenance_edge_row,
            )
            .optional()?
            .ok_or_else(|| Error::ObjectNotFound {
                kind: "provenance edge",
                id: edge_id.to_string(),
            })
    }

    pub fn list_session_provenance(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<(Vec<ProvenanceNode>, Vec<ProvenanceEdge>)> {
        self.list_session_provenance_page(session_id, 0, limit)
    }

    pub fn list_session_provenance_page(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<ProvenanceNode>, Vec<ProvenanceEdge>)> {
        self.lane_session(session_id)?;
        let limit = i64::try_from(limit.clamp(1, 10_000)).unwrap_or(10_000);
        let mut node_statement = self.conn.prepare(
            "SELECT provenance_node_id, lane_id, session_id, turn_id, node_kind,
                    summary, event_id, span_id, message_id, change_id, artifact_id,
                    source_confidence, classifier_version, created_at, attributes_json
             FROM lane_provenance_nodes WHERE session_id = ?1
             ORDER BY created_at, provenance_node_id LIMIT ?2 OFFSET ?3",
        )?;
        let nodes = node_statement
            .query_map(
                params![
                    session_id,
                    limit,
                    i64::try_from(offset.min(1_000_000)).unwrap_or(1_000_000)
                ],
                provenance_node_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut edge_statement = self.conn.prepare(
            "SELECT provenance_edge_id, lane_id, session_id, from_node_id,
                    to_node_id, relation, source_confidence, receipt_id,
                    created_at, attributes_json
             FROM lane_provenance_edges WHERE session_id = ?1
             ORDER BY created_at, provenance_edge_id LIMIT ?2 OFFSET ?3",
        )?;
        let edges = edge_statement
            .query_map(
                params![
                    session_id,
                    limit,
                    i64::try_from(offset.min(1_000_000)).unwrap_or(1_000_000)
                ],
                provenance_edge_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok((nodes, edges))
    }

    /// Materialize deterministic, explicitly derived activity labels from factual events.
    pub fn classify_session_activity(
        &mut self,
        session_id: &str,
        limit: usize,
    ) -> Result<ActivityClassificationReport> {
        const CLASSIFIER: &str = "trail-activity-rules/v1";
        let events = self.list_lane_events(None, Some(session_id), None, None, limit)?;
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        for event in events.into_iter().rev() {
            let Some((activity_kind, summary)) = classify_event_activity(&event.event_type) else {
                continue;
            };
            let source = self.create_provenance_node(ProvenanceNodeInput {
                session_id: session_id.to_string(),
                turn_id: event.turn_id.clone(),
                node_kind: "source_event".to_string(),
                summary: format!("Observed `{}` event", event.event_type),
                event_id: Some(event.event_id.clone()),
                span_id: None,
                message_id: event
                    .message_id
                    .as_ref()
                    .map(|message_id| message_id.0.clone()),
                change_id: event.change_id.as_ref().map(ToString::to_string),
                artifact_id: None,
                source_confidence: "factual".to_string(),
                classifier_version: None,
                attributes: Some(serde_json::json!({"event_type": event.event_type})),
            })?;
            let derived = self.create_provenance_node(ProvenanceNodeInput {
                session_id: session_id.to_string(),
                turn_id: event.turn_id,
                node_kind: activity_kind.to_string(),
                summary: summary.to_string(),
                event_id: None,
                span_id: None,
                message_id: None,
                change_id: None,
                artifact_id: None,
                source_confidence: "deterministic-derived".to_string(),
                classifier_version: Some(CLASSIFIER.to_string()),
                attributes: Some(serde_json::json!({
                    "rule": event.event_type,
                    "claims_hidden_reasoning": false,
                })),
            })?;
            let edge = self.create_provenance_edge(ProvenanceEdgeInput {
                from_node_id: derived.provenance_node_id.clone(),
                to_node_id: source.provenance_node_id.clone(),
                relation: "derived_from".to_string(),
                source_confidence: "deterministic-derived".to_string(),
                receipt_id: None,
                attributes: Some(serde_json::json!({"classifier_version": CLASSIFIER})),
            })?;
            nodes.push(source);
            nodes.push(derived);
            edges.push(edge);
        }
        nodes.sort_by(|left, right| left.provenance_node_id.cmp(&right.provenance_node_id));
        nodes.dedup_by(|left, right| left.provenance_node_id == right.provenance_node_id);
        edges.sort_by(|left, right| left.provenance_edge_id.cmp(&right.provenance_edge_id));
        edges.dedup_by(|left, right| left.provenance_edge_id == right.provenance_edge_id);
        Ok(ActivityClassificationReport {
            session_id: session_id.to_string(),
            classifier_version: CLASSIFIER.to_string(),
            nodes,
            edges,
        })
    }

    pub fn create_session_attestation(
        &mut self,
        session_id: &str,
        capture_policy: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<SessionAttestation> {
        validate_evidence_text("attestation capture policy", capture_policy, 128)?;
        let session = self.lane_session(session_id)?;
        let turns = self.lane_session_turns(session_id)?;
        for turn in &turns {
            if turn.ended_at.is_some() {
                self.create_turn_evidence_manifest(&turn.turn_id)?;
            }
        }
        let previous = self.latest_session_attestation(session_id)?;
        let mut covered = std::collections::BTreeSet::new();
        {
            let mut statement = self.conn.prepare(
                "SELECT turn_id FROM lane_session_attestation_turns
                 WHERE attestation_id IN (
                   SELECT attestation_id FROM lane_session_attestations WHERE session_id = ?1
                 )",
            )?;
            for turn_id in
                statement.query_map(params![session_id], |row| row.get::<_, String>(0))?
            {
                covered.insert(turn_id?);
            }
        }
        let manifests = self
            .list_turn_evidence_manifests(session_id, 1_000)?
            .into_iter()
            .filter(|manifest| !covered.contains(&manifest.turn_id))
            .collect::<Vec<_>>();
        if manifests.is_empty() {
            return previous.ok_or_else(|| {
                Error::InvalidInput(format!(
                    "session `{session_id}` has no completed turns to attest"
                ))
            });
        }
        let capture_run_id = self
            .conn
            .query_row(
                "SELECT capture_run_id FROM lane_agent_sessions
                 WHERE trail_session_id = ?1 ORDER BY created_at LIMIT 1",
                params![session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let attested_turns = manifests
            .iter()
            .map(|manifest| SessionAttestationTurn {
                turn_id: manifest.turn_id.clone(),
                change_id: (manifest.statement.after_change != manifest.statement.before_change)
                    .then(|| manifest.statement.after_change.0.clone()),
                evidence_manifest_id: manifest.manifest_id.clone(),
                evidence_digest: manifest.digest.clone(),
            })
            .collect::<Vec<_>>();
        let statement = SessionAttestationStatement {
            schema: SESSION_ATTESTATION_SCHEMA.to_string(),
            version: SESSION_ATTESTATION_VERSION,
            workspace_id: self.config.workspace.id.0.clone(),
            lane_id: session.lane_id.clone(),
            session_id: session_id.to_string(),
            capture_run_id: capture_run_id.clone(),
            previous_attestation_id: previous
                .as_ref()
                .map(|attestation| attestation.attestation_id.clone()),
            turns: attested_turns.clone(),
            capture_policy: capture_policy.to_string(),
        };
        let statement_bytes = canonical_json_bytes(&statement)?;
        let statement_digest = digest_bytes(&statement_bytes);
        let attestation_id = format!(
            "attestation_{}",
            crate::ids::short_hash(statement_digest.as_bytes(), 24)
        );
        let object_id = self.put_object(
            SESSION_ATTESTATION_OBJECT_KIND,
            SESSION_ATTESTATION_VERSION,
            &statement,
        )?;
        let metadata_json = metadata.as_ref().map(canonical_json_string).transpose()?;
        let now = now_millis();
        let _lock = self.acquire_write_lock()?;
        self.conn
            .execute_batch("SAVEPOINT create_session_attestation")?;
        let result = (|| -> Result<()> {
            self.conn.execute(
                "INSERT INTO lane_session_attestations
                 (attestation_id, lane_id, session_id, capture_run_id,
                  previous_attestation_id, statement_object_id, statement_digest,
                  signature_json, status, created_at, superseded_by, metadata_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, 'unsigned', ?8, NULL, ?9)
                 ON CONFLICT(attestation_id) DO NOTHING",
                params![
                    attestation_id,
                    session.lane_id,
                    session_id,
                    capture_run_id,
                    statement.previous_attestation_id,
                    object_id.0,
                    statement_digest,
                    now,
                    metadata_json,
                ],
            )?;
            for turn in &attested_turns {
                self.conn.execute(
                    "INSERT INTO lane_session_attestation_turns
                     (attestation_id, turn_id, change_id, evidence_manifest_id)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(attestation_id, turn_id) DO NOTHING",
                    params![
                        attestation_id,
                        turn.turn_id,
                        turn.change_id,
                        turn.evidence_manifest_id,
                    ],
                )?;
            }
            self.conn.execute(
                "UPDATE lane_agent_sessions SET last_attestation_id = ?2, updated_at = ?3
                 WHERE trail_session_id = ?1",
                params![session_id, attestation_id, now],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => self
                .conn
                .execute_batch("RELEASE SAVEPOINT create_session_attestation")?,
            Err(error) => {
                self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT create_session_attestation;
                     RELEASE SAVEPOINT create_session_attestation",
                )?;
                return Err(error);
            }
        }
        self.session_attestation(&attestation_id)
    }

    pub fn session_attestation(&self, attestation_id: &str) -> Result<SessionAttestation> {
        let row = self
            .conn
            .query_row(
                SESSION_ATTESTATION_SELECT_BY_ID,
                params![attestation_id],
                session_attestation_row,
            )
            .optional()?
            .ok_or_else(|| Error::ObjectNotFound {
                kind: "session attestation",
                id: attestation_id.to_string(),
            })?;
        self.materialize_session_attestation(row)
    }

    pub fn list_session_attestations(&self, session_id: &str) -> Result<Vec<SessionAttestation>> {
        self.list_session_attestations_page(session_id, 0, 1_000)
    }

    pub fn list_session_attestations_page(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<SessionAttestation>> {
        self.lane_session(session_id)?;
        let mut statement = self.conn.prepare(
            "SELECT attestation_id, lane_id, session_id, capture_run_id,
                    previous_attestation_id, statement_object_id, statement_digest,
                    signature_json, status, created_at, superseded_by, metadata_json
             FROM lane_session_attestations WHERE session_id = ?1
             ORDER BY created_at, attestation_id LIMIT ?2 OFFSET ?3",
        )?;
        let rows = statement
            .query_map(
                params![
                    session_id,
                    i64::try_from(limit.clamp(1, 1_000)).unwrap_or(1_000),
                    i64::try_from(offset.min(1_000_000)).unwrap_or(1_000_000)
                ],
                session_attestation_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|row| self.materialize_session_attestation(row))
            .collect()
    }

    pub fn verify_session_attestation(
        &self,
        attestation_id: &str,
    ) -> Result<AttestationVerificationReport> {
        let attestation = self.session_attestation(attestation_id)?;
        let statement: SessionAttestationStatement = self.get_object(
            SESSION_ATTESTATION_OBJECT_KIND,
            &attestation.statement_object_id,
        )?;
        let statement_digest_valid =
            digest_bytes(&canonical_json_bytes(&statement)?) == attestation.statement_digest;
        let mut diagnostics = Vec::new();
        if !statement_digest_valid {
            diagnostics.push("attestation statement digest mismatch".to_string());
        }
        let mut evidence_digests_valid = true;
        for turn in &statement.turns {
            match self.turn_evidence_manifest(&turn.turn_id) {
                Ok(manifest)
                    if manifest.manifest_id == turn.evidence_manifest_id
                        && manifest.digest == turn.evidence_digest => {}
                _ => {
                    evidence_digests_valid = false;
                    diagnostics.push(format!(
                        "turn `{}` evidence manifest no longer verifies",
                        turn.turn_id
                    ));
                }
            }
        }
        let chain_valid = self.verify_attestation_chain(&attestation, 1_000)?;
        if !chain_valid {
            diagnostics.push("attestation predecessor chain is invalid".to_string());
        }
        let (signature_status, signature_valid) =
            self.verify_attestation_signature(&attestation, &mut diagnostics)?;
        let valid =
            statement_digest_valid && evidence_digests_valid && chain_valid && signature_valid;
        Ok(AttestationVerificationReport {
            attestation_id: attestation_id.to_string(),
            statement_digest_valid,
            evidence_digests_valid,
            chain_valid,
            signature_status,
            valid,
            diagnostics,
        })
    }

    pub fn sign_session_attestation(
        &mut self,
        attestation_id: &str,
        secret_key: &[u8; 32],
    ) -> Result<SessionAttestation> {
        let attestation = self.session_attestation(attestation_id)?;
        let signing_key = SigningKey::from_bytes(secret_key);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.as_bytes());
        let key_id = attestation_key_id(verifying_key.as_bytes());
        if self.attestation_key_revocation(&key_id)?.is_some() {
            return Err(Error::Conflict(format!(
                "attestation signing key `{key_id}` is revoked"
            )));
        }
        let signature = signing_key.sign(attestation.statement_digest.as_bytes());
        let envelope = AttestationSignature {
            algorithm: "ed25519".to_string(),
            key_id,
            public_key_hex,
            signature_hex: hex::encode(signature.to_bytes()),
        };
        let signature_json = serde_json::to_string(&envelope)?;
        let _lock = self.acquire_write_lock()?;
        self.conn.execute(
            "UPDATE lane_session_attestations
             SET signature_json = ?2, status = 'signed'
             WHERE attestation_id = ?1",
            params![attestation_id, signature_json],
        )?;
        self.session_attestation(attestation_id)
    }

    pub fn revoke_attestation_key(
        &mut self,
        public_key: &[u8; 32],
        reason: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<AttestationKeyRevocation> {
        validate_evidence_text("attestation key revocation reason", reason, 4_096)?;
        VerifyingKey::from_bytes(public_key)
            .map_err(|error| Error::InvalidInput(format!("invalid Ed25519 public key: {error}")))?;
        let key_id = attestation_key_id(public_key);
        let public_key_hex = hex::encode(public_key);
        let metadata_json = metadata.as_ref().map(canonical_json_string).transpose()?;
        let now = now_millis();
        let _lock = self.acquire_write_lock()?;
        self.conn.execute(
            "INSERT INTO agent_attestation_key_revocations
             (key_id, public_key_hex, reason, revoked_at, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(key_id) DO UPDATE SET
               reason = excluded.reason,
               revoked_at = excluded.revoked_at,
               metadata_json = excluded.metadata_json",
            params![key_id, public_key_hex, reason, now, metadata_json],
        )?;
        self.attestation_key_revocation(&key_id)?
            .ok_or_else(|| Error::Corrupt(format!("revocation `{key_id}` was not persisted")))
    }

    pub fn attestation_key_revocation(
        &self,
        key_id: &str,
    ) -> Result<Option<AttestationKeyRevocation>> {
        self.conn
            .query_row(
                "SELECT key_id, public_key_hex, reason, revoked_at, metadata_json
                 FROM agent_attestation_key_revocations WHERE key_id = ?1",
                params![key_id],
                attestation_key_revocation_row,
            )
            .optional()
            .map_err(Error::from)
    }

    fn verify_attestation_signature(
        &self,
        attestation: &SessionAttestation,
        diagnostics: &mut Vec<String>,
    ) -> Result<(String, bool)> {
        let Some(signature_value) = attestation.signature.clone() else {
            return Ok(("unsigned".to_string(), true));
        };
        let envelope: AttestationSignature = match serde_json::from_value(signature_value) {
            Ok(envelope) => envelope,
            Err(error) => {
                diagnostics.push(format!("invalid attestation signature envelope: {error}"));
                return Ok(("invalid-envelope".to_string(), false));
            }
        };
        if envelope.algorithm != "ed25519" {
            diagnostics.push(format!(
                "unsupported attestation signature algorithm `{}`",
                envelope.algorithm
            ));
            return Ok(("unsupported-algorithm".to_string(), false));
        }
        let public_key = decode_fixed_hex::<32>(&envelope.public_key_hex, "public key")?;
        let signature_bytes = decode_fixed_hex::<64>(&envelope.signature_hex, "signature")?;
        if attestation_key_id(&public_key) != envelope.key_id {
            diagnostics.push("attestation key id does not match public key".to_string());
            return Ok(("invalid-key-id".to_string(), false));
        }
        if self.attestation_key_revocation(&envelope.key_id)?.is_some() {
            diagnostics.push(format!(
                "attestation key `{}` has been revoked",
                envelope.key_id
            ));
            return Ok(("revoked".to_string(), false));
        }
        let verifying_key = VerifyingKey::from_bytes(&public_key).map_err(|error| {
            Error::InvalidInput(format!("invalid attestation public key: {error}"))
        })?;
        let signature = Signature::from_bytes(&signature_bytes);
        match verifying_key.verify(attestation.statement_digest.as_bytes(), &signature) {
            Ok(()) => Ok(("valid".to_string(), true)),
            Err(error) => {
                diagnostics.push(format!(
                    "attestation signature verification failed: {error}"
                ));
                Ok(("invalid".to_string(), false))
            }
        }
    }

    fn latest_session_attestation(&self, session_id: &str) -> Result<Option<SessionAttestation>> {
        let row = self
            .conn
            .query_row(
                "SELECT attestation_id, lane_id, session_id, capture_run_id,
                        previous_attestation_id, statement_object_id, statement_digest,
                        signature_json, status, created_at, superseded_by, metadata_json
                 FROM lane_session_attestations WHERE session_id = ?1
                 ORDER BY created_at DESC, attestation_id DESC LIMIT 1",
                params![session_id],
                session_attestation_row,
            )
            .optional()?;
        row.map(|row| self.materialize_session_attestation(row))
            .transpose()
    }

    fn materialize_session_attestation(
        &self,
        row: SessionAttestationRow,
    ) -> Result<SessionAttestation> {
        let statement: SessionAttestationStatement =
            self.get_object(SESSION_ATTESTATION_OBJECT_KIND, &row.statement_object_id)?;
        if statement.schema != SESSION_ATTESTATION_SCHEMA
            || statement.version != SESSION_ATTESTATION_VERSION
            || statement.session_id != row.session_id
            || statement.lane_id != row.lane_id
            || digest_bytes(&canonical_json_bytes(&statement)?) != row.statement_digest
        {
            return Err(Error::Corrupt(format!(
                "session attestation `{}` statement is invalid",
                row.attestation_id
            )));
        }
        Ok(SessionAttestation {
            attestation_id: row.attestation_id,
            lane_id: row.lane_id,
            session_id: row.session_id,
            capture_run_id: row.capture_run_id,
            previous_attestation_id: row.previous_attestation_id,
            statement_object_id: row.statement_object_id,
            statement_digest: row.statement_digest,
            signature: row.signature,
            status: row.status,
            created_at: row.created_at,
            superseded_by: row.superseded_by,
            metadata: row.metadata,
            turns: statement.turns,
        })
    }

    fn verify_attestation_chain(
        &self,
        attestation: &SessionAttestation,
        max_depth: usize,
    ) -> Result<bool> {
        let mut current = attestation.clone();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..max_depth {
            if !seen.insert(current.attestation_id.clone()) {
                return Ok(false);
            }
            let Some(previous_id) = current.previous_attestation_id.as_deref() else {
                return Ok(true);
            };
            let previous = self.session_attestation(previous_id)?;
            if previous.session_id != attestation.session_id
                || previous.created_at > current.created_at
            {
                return Ok(false);
            }
            current = previous;
        }
        Ok(false)
    }

    pub fn propose_learning(&mut self, input: LearningInput) -> Result<Learning> {
        validate_evidence_text("learning scope", &input.scope, 128)?;
        validate_evidence_text("learning body", &input.body, 32 * 1024)?;
        if input
            .confidence
            .is_some_and(|confidence| !(0.0..=1.0).contains(&confidence))
        {
            return Err(Error::InvalidInput(
                "learning confidence must be between 0 and 1".to_string(),
            ));
        }
        let session = self.lane_session(&input.session_id)?;
        validate_optional_turn_for_session(self, input.turn_id.as_deref(), &input.session_id)?;
        if let Some(artifact_id) = input.source_artifact_id.as_deref() {
            let artifact = self.lane_artifact(artifact_id)?;
            if artifact.session_id != input.session_id {
                return Err(Error::InvalidInput(format!(
                    "learning artifact `{artifact_id}` does not belong to session `{}`",
                    input.session_id
                )));
            }
        }
        let body = redact_sensitive_text(&input.body);
        let anchor_json = input
            .anchor
            .as_ref()
            .map(canonical_json_string)
            .transpose()?;
        let metadata_json = input
            .metadata
            .as_ref()
            .map(canonical_json_string)
            .transpose()?;
        let identity = canonical_json_bytes(&serde_json::json!({
            "session_id": input.session_id,
            "turn_id": input.turn_id,
            "scope": input.scope,
            "body": body,
            "source_artifact_id": input.source_artifact_id,
            "anchor": input.anchor,
        }))?;
        let learning_id = format!("learning_{}", crate::ids::short_hash(&identity, 24));
        let now = now_millis();
        let _lock = self.acquire_write_lock()?;
        self.conn.execute(
            "INSERT INTO lane_learnings
             (learning_id, lane_id, session_id, turn_id, scope, body, status,
              confidence, source_artifact_id, anchor_json, created_at, reviewed_at,
              reviewer, expires_at, superseded_by, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'proposed', ?7, ?8, ?9, ?10,
                     NULL, NULL, ?11, NULL, ?12)
             ON CONFLICT(learning_id) DO NOTHING",
            params![
                learning_id,
                session.lane_id,
                input.session_id,
                input.turn_id,
                input.scope,
                body,
                input.confidence,
                input.source_artifact_id,
                anchor_json,
                now,
                input.expires_at,
                metadata_json,
            ],
        )?;
        self.learning(&learning_id)
    }

    pub fn review_learning(
        &mut self,
        learning_id: &str,
        accept: bool,
        reviewer: &str,
    ) -> Result<Learning> {
        validate_evidence_text("learning reviewer", reviewer, 256)?;
        let existing = self.learning(learning_id)?;
        if !matches!(
            existing.status.as_str(),
            "proposed" | "accepted" | "rejected"
        ) {
            return Err(Error::Conflict(format!(
                "learning `{learning_id}` cannot be reviewed from status `{}`",
                existing.status
            )));
        }
        let status = if accept { "accepted" } else { "rejected" };
        let _lock = self.acquire_write_lock()?;
        self.conn.execute(
            "UPDATE lane_learnings SET status = ?2, reviewed_at = ?3, reviewer = ?4
             WHERE learning_id = ?1",
            params![learning_id, status, now_millis(), reviewer],
        )?;
        self.learning(learning_id)
    }

    pub fn learning(&self, learning_id: &str) -> Result<Learning> {
        self.conn
            .query_row(LEARNING_SELECT_BY_ID, params![learning_id], learning_row)
            .optional()?
            .ok_or_else(|| Error::ObjectNotFound {
                kind: "learning",
                id: learning_id.to_string(),
            })
    }

    pub fn list_learnings(
        &self,
        session_id: Option<&str>,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Learning>> {
        self.list_learnings_page(session_id, status, 0, limit)
    }

    pub fn list_learnings_page(
        &self,
        session_id: Option<&str>,
        status: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Learning>> {
        if let Some(session_id) = session_id {
            self.lane_session(session_id)?;
        }
        if let Some(status) = status {
            if !matches!(
                status,
                "proposed" | "accepted" | "rejected" | "expired" | "superseded"
            ) {
                return Err(Error::InvalidInput(format!(
                    "unknown learning status `{status}`"
                )));
            }
        }
        let mut statement = self.conn.prepare(
            "SELECT learning_id, lane_id, session_id, turn_id, scope, body, status,
                    confidence, source_artifact_id, anchor_json, created_at, reviewed_at,
                    reviewer, expires_at, superseded_by, metadata_json
             FROM lane_learnings
             WHERE (?1 IS NULL OR session_id = ?1) AND (?2 IS NULL OR status = ?2)
             ORDER BY created_at DESC, learning_id DESC LIMIT ?3 OFFSET ?4",
        )?;
        let learnings = statement
            .query_map(
                params![
                    session_id,
                    status,
                    i64::try_from(limit.clamp(1, 1_000)).unwrap_or(1_000),
                    i64::try_from(offset.min(1_000_000)).unwrap_or(1_000_000)
                ],
                learning_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(learnings)
    }

    pub fn supersede_learning(
        &mut self,
        learning_id: &str,
        superseded_by: &str,
        reviewer: &str,
    ) -> Result<Learning> {
        validate_evidence_text("learning reviewer", reviewer, 256)?;
        let existing = self.learning(learning_id)?;
        let replacement = self.learning(superseded_by)?;
        if existing.lane_id != replacement.lane_id || existing.scope != replacement.scope {
            return Err(Error::InvalidInput(
                "a learning may only be superseded by one in the same lane and scope".to_string(),
            ));
        }
        if learning_id == superseded_by {
            return Err(Error::InvalidInput(
                "a learning cannot supersede itself".to_string(),
            ));
        }
        let _lock = self.acquire_write_lock()?;
        self.conn.execute(
            "UPDATE lane_learnings
             SET status = 'superseded', superseded_by = ?2, reviewed_at = ?3, reviewer = ?4
             WHERE learning_id = ?1",
            params![learning_id, superseded_by, now_millis(), reviewer],
        )?;
        self.learning(learning_id)
    }

    pub fn expire_learnings(&mut self, at_millis: i64) -> Result<usize> {
        if at_millis < 0 {
            return Err(Error::InvalidInput(
                "learning expiry timestamp must be non-negative".to_string(),
            ));
        }
        let _lock = self.acquire_write_lock()?;
        self.conn
            .execute(
                "UPDATE lane_learnings SET status = 'expired', reviewed_at = ?1
                 WHERE expires_at IS NOT NULL AND expires_at <= ?1
                   AND status IN ('proposed', 'accepted')",
                params![at_millis],
            )
            .map_err(Error::from)
    }

    /// Resolve explicitly requested, reviewed learning context under a strict byte bound.
    pub fn accepted_learning_context(
        &self,
        lane: &str,
        scopes: &[String],
        max_bytes: usize,
    ) -> Result<Vec<Learning>> {
        if scopes.is_empty() || max_bytes == 0 || max_bytes > 1024 * 1024 {
            return Err(Error::InvalidInput(
                "accepted learning context requires scopes and a 1..=1048576 byte bound"
                    .to_string(),
            ));
        }
        let branch = self.lane_branch(lane)?;
        let now = now_millis();
        let mut statement = self.conn.prepare(
            "SELECT learning_id, lane_id, session_id, turn_id, scope, body, status,
                    confidence, source_artifact_id, anchor_json, created_at, reviewed_at,
                    reviewer, expires_at, superseded_by, metadata_json
             FROM lane_learnings
             WHERE lane_id = ?1 AND status = 'accepted'
               AND (expires_at IS NULL OR expires_at > ?2)
             ORDER BY confidence DESC, created_at DESC, learning_id",
        )?;
        let candidates = statement
            .query_map(params![branch.lane_id, now], learning_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let allowed = scopes
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        let mut total = 0usize;
        let mut selected = Vec::new();
        for learning in candidates {
            if !allowed.contains(learning.scope.as_str()) {
                continue;
            }
            let size = learning.body.len();
            if total.saturating_add(size) > max_bytes {
                continue;
            }
            total += size;
            selected.push(learning);
        }
        Ok(selected)
    }

    pub fn link_git_commit_to_agent(&mut self, input: GitAgentLinkInput) -> Result<GitAgentLink> {
        if !matches!(input.git_commit.len(), 40 | 64)
            || !input
                .git_commit
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(Error::InvalidInput(
                "git agent link commit must be a full 40- or 64-character hexadecimal object id"
                    .to_string(),
            ));
        }
        validate_evidence_text("git link confidence", &input.confidence, 128)?;
        validate_evidence_text("git link source", &input.source, 128)?;
        let session = self.lane_session(&input.session_id)?;
        validate_optional_turn_for_session(self, input.turn_id.as_deref(), &input.session_id)?;
        for change in [
            input.from_change.as_deref(),
            input.through_change.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            self.operation(&ChangeId(change.to_string()))?;
        }
        if input.from_change.is_none() && input.through_change.is_none() {
            return Err(Error::InvalidInput(
                "git agent link requires an exact from_change or through_change identity"
                    .to_string(),
            ));
        }
        let metadata_json = input
            .metadata
            .as_ref()
            .map(canonical_json_string)
            .transpose()?;
        let identity = format!(
            "{}:{}:{}:{}",
            input.git_commit.to_ascii_lowercase(),
            input.session_id,
            input.turn_id.as_deref().unwrap_or(""),
            input.source
        );
        let link_id = format!(
            "git_agent_link_{}",
            crate::ids::short_hash(identity.as_bytes(), 24)
        );
        let _lock = self.acquire_write_lock()?;
        self.conn.execute(
            "INSERT INTO git_agent_links
             (git_agent_link_id, git_commit, lane_id, session_id, turn_id,
              from_change, through_change, confidence, source, created_at, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(git_commit, session_id, COALESCE(turn_id, ''), source) DO NOTHING",
            params![
                link_id,
                input.git_commit.to_ascii_lowercase(),
                session.lane_id,
                input.session_id,
                input.turn_id,
                input.from_change,
                input.through_change,
                input.confidence,
                input.source,
                now_millis(),
                metadata_json,
            ],
        )?;
        self.git_agent_link(&link_id).or_else(|_| {
            self.conn
                .query_row(
                    "SELECT git_agent_link_id, git_commit, lane_id, session_id, turn_id,
                        from_change, through_change, confidence, source, created_at, metadata_json
                 FROM git_agent_links WHERE git_commit = ?1 AND session_id = ?2
                   AND COALESCE(turn_id, '') = COALESCE(?3, '') AND source = ?4",
                    params![
                        input.git_commit.to_ascii_lowercase(),
                        input.session_id,
                        input.turn_id,
                        input.source
                    ],
                    git_agent_link_row,
                )
                .map_err(Error::from)
        })
    }

    pub fn git_agent_link(&self, link_id: &str) -> Result<GitAgentLink> {
        self.conn
            .query_row(
                GIT_AGENT_LINK_SELECT_BY_ID,
                params![link_id],
                git_agent_link_row,
            )
            .optional()?
            .ok_or_else(|| Error::ObjectNotFound {
                kind: "git agent link",
                id: link_id.to_string(),
            })
    }

    pub fn list_git_agent_links(&self, session_id: &str) -> Result<Vec<GitAgentLink>> {
        self.list_git_agent_links_page(session_id, 0, 1_000)
    }

    pub fn list_git_agent_links_page(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<GitAgentLink>> {
        self.lane_session(session_id)?;
        let mut statement = self.conn.prepare(
            "SELECT git_agent_link_id, git_commit, lane_id, session_id, turn_id,
                    from_change, through_change, confidence, source, created_at, metadata_json
             FROM git_agent_links WHERE session_id = ?1
             ORDER BY created_at, git_agent_link_id LIMIT ?2 OFFSET ?3",
        )?;
        let links = statement
            .query_map(
                params![
                    session_id,
                    i64::try_from(limit.clamp(1, 1_000)).unwrap_or(1_000),
                    i64::try_from(offset.min(1_000_000)).unwrap_or(1_000_000)
                ],
                git_agent_link_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(links)
    }

    pub fn export_agent_trace(
        &self,
        session_id: &str,
        include_attachments: bool,
    ) -> Result<PortableAgentTrace> {
        let session = self.lane_session(session_id)?;
        ensure_export_bound(
            &self.conn,
            "lane_turn_evidence_manifests",
            session_id,
            1_000,
        )?;
        ensure_export_bound(&self.conn, "lane_artifacts", session_id, 1_000)?;
        ensure_export_bound(&self.conn, "lane_provenance_nodes", session_id, 10_000)?;
        ensure_export_bound(&self.conn, "lane_provenance_edges", session_id, 10_000)?;
        ensure_export_bound(&self.conn, "lane_session_attestations", session_id, 1_000)?;
        ensure_export_bound(&self.conn, "lane_learnings", session_id, 1_000)?;
        ensure_export_bound(&self.conn, "git_agent_links", session_id, 1_000)?;

        let mut evidence_manifests = self.list_turn_evidence_manifests(session_id, 1_000)?;
        evidence_manifests.sort_by(|left, right| left.turn_id.cmp(&right.turn_id));
        let mut artifacts = self
            .list_lane_artifacts(session_id, None, 1_000)?
            .into_iter()
            .map(|artifact| {
                let content = include_attachments
                    .then(|| self.lane_artifact_content(&artifact.artifact_id))
                    .transpose()?;
                Ok(PortableAgentArtifact {
                    artifact_id: artifact.artifact_id,
                    provider: artifact.provider,
                    artifact_kind: artifact.artifact_kind,
                    format: artifact.format,
                    source: artifact.source,
                    content_digest: artifact.content_digest,
                    size_bytes: artifact.size_bytes,
                    start_offset: artifact.start_offset,
                    end_offset: artifact.end_offset,
                    trust: artifact.trust,
                    retention_status: artifact.retention_status,
                    content,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        artifacts.sort_by(|left, right| left.artifact_id.cmp(&right.artifact_id));
        let (mut provenance_nodes, mut provenance_edges) =
            self.list_session_provenance(session_id, 10_000)?;
        provenance_nodes
            .sort_by(|left, right| left.provenance_node_id.cmp(&right.provenance_node_id));
        provenance_edges
            .sort_by(|left, right| left.provenance_edge_id.cmp(&right.provenance_edge_id));
        let mut attestations = self.list_session_attestations(session_id)?;
        attestations.sort_by(|left, right| left.attestation_id.cmp(&right.attestation_id));
        let mut learnings = self.list_learnings(Some(session_id), None, 1_000)?;
        learnings.sort_by(|left, right| left.learning_id.cmp(&right.learning_id));
        let mut git_links = self.list_git_agent_links(session_id)?;
        git_links.sort_by(|left, right| left.git_agent_link_id.cmp(&right.git_agent_link_id));
        let trace = PortableAgentTrace {
            schema: AGENT_TRACE_SCHEMA.to_string(),
            version: AGENT_TRACE_VERSION,
            source_workspace_id: self.config.workspace.id.0.clone(),
            lane_id: session.lane_id,
            session_id: session_id.to_string(),
            session_status: session.status,
            evidence_manifests,
            artifacts,
            provenance_nodes,
            provenance_edges,
            attestations,
            learnings,
            git_links,
        };
        let verification = trace.verify();
        if !verification.valid {
            return Err(Error::Corrupt(format!(
                "generated portable trace failed verification: {}",
                verification.diagnostics.join("; ")
            )));
        }
        Ok(trace)
    }
}

struct EvidenceManifestRow {
    manifest_id: String,
    lane_id: String,
    session_id: String,
    turn_id: String,
    schema_version: u16,
    object_id: ObjectId,
    digest: String,
    created_at: i64,
}

fn classify_event_activity(event_type: &str) -> Option<(&'static str, &'static str)> {
    if event_type.starts_with("tool.") || event_type.starts_with("tool_") {
        Some(("tool_activity", "Agent used a tool"))
    } else if event_type.starts_with("workspace.") || event_type.contains("checkpoint") {
        Some((
            "workspace_activity",
            "Agent changed or checkpointed workspace state",
        ))
    } else if event_type.starts_with("plan.") || event_type.contains("plan_update") {
        Some(("plan_activity", "Agent updated its explicit plan"))
    } else if event_type.starts_with("approval.") || event_type.contains("permission") {
        Some((
            "approval_activity",
            "Agent participated in an approval decision",
        ))
    } else if event_type.starts_with("message.") || event_type.contains("message_flushed") {
        Some((
            "communication_activity",
            "Agent exchanged a captured message",
        ))
    } else {
        None
    }
}

const PROVENANCE_NODE_SELECT_BY_ID: &str =
    "SELECT provenance_node_id, lane_id, session_id, turn_id, node_kind,
            summary, event_id, span_id, message_id, change_id, artifact_id,
            source_confidence, classifier_version, created_at, attributes_json
     FROM lane_provenance_nodes WHERE provenance_node_id = ?1";

fn provenance_node_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProvenanceNode> {
    Ok(ProvenanceNode {
        provenance_node_id: row.get(0)?,
        lane_id: row.get(1)?,
        session_id: row.get(2)?,
        turn_id: row.get(3)?,
        node_kind: row.get(4)?,
        summary: row.get(5)?,
        event_id: row.get(6)?,
        span_id: row.get(7)?,
        message_id: row.get(8)?,
        change_id: row.get(9)?,
        artifact_id: row.get(10)?,
        source_confidence: row.get(11)?,
        classifier_version: row.get(12)?,
        created_at: row.get(13)?,
        attributes: parse_optional_json_column(row, 14)?,
    })
}

fn provenance_edge_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProvenanceEdge> {
    Ok(ProvenanceEdge {
        provenance_edge_id: row.get(0)?,
        lane_id: row.get(1)?,
        session_id: row.get(2)?,
        from_node_id: row.get(3)?,
        to_node_id: row.get(4)?,
        relation: row.get(5)?,
        source_confidence: row.get(6)?,
        receipt_id: row.get(7)?,
        created_at: row.get(8)?,
        attributes: parse_optional_json_column(row, 9)?,
    })
}

const SESSION_ATTESTATION_SELECT_BY_ID: &str =
    "SELECT attestation_id, lane_id, session_id, capture_run_id,
            previous_attestation_id, statement_object_id, statement_digest,
            signature_json, status, created_at, superseded_by, metadata_json
     FROM lane_session_attestations WHERE attestation_id = ?1";

struct SessionAttestationRow {
    attestation_id: String,
    lane_id: String,
    session_id: String,
    capture_run_id: Option<String>,
    previous_attestation_id: Option<String>,
    statement_object_id: ObjectId,
    statement_digest: String,
    signature: Option<serde_json::Value>,
    status: String,
    created_at: i64,
    superseded_by: Option<String>,
    metadata: Option<serde_json::Value>,
}

fn session_attestation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionAttestationRow> {
    Ok(SessionAttestationRow {
        attestation_id: row.get(0)?,
        lane_id: row.get(1)?,
        session_id: row.get(2)?,
        capture_run_id: row.get(3)?,
        previous_attestation_id: row.get(4)?,
        statement_object_id: ObjectId(row.get(5)?),
        statement_digest: row.get(6)?,
        signature: parse_optional_json_column(row, 7)?,
        status: row.get(8)?,
        created_at: row.get(9)?,
        superseded_by: row.get(10)?,
        metadata: parse_optional_json_column(row, 11)?,
    })
}

const LEARNING_SELECT_BY_ID: &str =
    "SELECT learning_id, lane_id, session_id, turn_id, scope, body, status,
            confidence, source_artifact_id, anchor_json, created_at, reviewed_at,
            reviewer, expires_at, superseded_by, metadata_json
     FROM lane_learnings WHERE learning_id = ?1";

fn learning_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Learning> {
    Ok(Learning {
        learning_id: row.get(0)?,
        lane_id: row.get(1)?,
        session_id: row.get(2)?,
        turn_id: row.get(3)?,
        scope: row.get(4)?,
        body: row.get(5)?,
        status: row.get(6)?,
        confidence: row.get(7)?,
        source_artifact_id: row.get(8)?,
        anchor: parse_optional_json_column(row, 9)?,
        created_at: row.get(10)?,
        reviewed_at: row.get(11)?,
        reviewer: row.get(12)?,
        expires_at: row.get(13)?,
        superseded_by: row.get(14)?,
        metadata: parse_optional_json_column(row, 15)?,
    })
}

const GIT_AGENT_LINK_SELECT_BY_ID: &str =
    "SELECT git_agent_link_id, git_commit, lane_id, session_id, turn_id,
            from_change, through_change, confidence, source, created_at, metadata_json
     FROM git_agent_links WHERE git_agent_link_id = ?1";

fn git_agent_link_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GitAgentLink> {
    Ok(GitAgentLink {
        git_agent_link_id: row.get(0)?,
        git_commit: row.get(1)?,
        lane_id: row.get(2)?,
        session_id: row.get(3)?,
        turn_id: row.get(4)?,
        from_change: row.get(5)?,
        through_change: row.get(6)?,
        confidence: row.get(7)?,
        source: row.get(8)?,
        created_at: row.get(9)?,
        metadata: parse_optional_json_column(row, 10)?,
    })
}

fn attestation_key_revocation_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<AttestationKeyRevocation> {
    Ok(AttestationKeyRevocation {
        key_id: row.get(0)?,
        public_key_hex: row.get(1)?,
        reason: row.get(2)?,
        revoked_at: row.get(3)?,
        metadata: parse_optional_json_column(row, 4)?,
    })
}

fn parse_optional_json_column(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<serde_json::Value>> {
    row.get::<_, Option<String>>(index)?
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

fn validate_optional_turn_for_session(
    db: &Trail,
    turn_id: Option<&str>,
    session_id: &str,
) -> Result<()> {
    if let Some(turn_id) = turn_id {
        let turn = db.lane_turn(turn_id)?;
        if turn.session_id.as_deref() != Some(session_id) {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` does not belong to session `{session_id}`"
            )));
        }
    }
    Ok(())
}

fn validate_evidence_text(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(Error::InvalidInput(format!(
            "{name} must contain 1..={max_bytes} non-control bytes"
        )));
    }
    Ok(())
}

fn canonical_json_string(value: &serde_json::Value) -> Result<String> {
    serde_json::to_string(value).map_err(Error::from)
}

fn attestation_key_id(public_key: &[u8; 32]) -> String {
    let digest = Sha256::digest(public_key);
    format!("ed25519_{}", hex::encode(&digest[..16]))
}

fn decode_fixed_hex<const N: usize>(value: &str, name: &str) -> Result<[u8; N]> {
    let bytes = hex::decode(value)
        .map_err(|error| Error::InvalidInput(format!("invalid {name} hex: {error}")))?;
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        Error::InvalidInput(format!(
            "invalid {name} length {}; expected {N} bytes",
            bytes.len()
        ))
    })
}

fn ensure_export_bound(
    conn: &rusqlite::Connection,
    table: &str,
    session_id: &str,
    maximum: i64,
) -> Result<()> {
    let count: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE session_id = ?1"),
        params![session_id],
        |row| row.get(0),
    )?;
    if count > maximum {
        return Err(Error::InvalidInput(format!(
            "session `{session_id}` has {count} rows in {table}; bounded export maximum is {maximum}"
        )));
    }
    Ok(())
}

fn evidence_manifest_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EvidenceManifestRow> {
    let raw_version: i64 = row.get(4)?;
    let schema_version = u16::try_from(raw_version).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(EvidenceManifestRow {
        manifest_id: row.get(0)?,
        lane_id: row.get(1)?,
        session_id: row.get(2)?,
        turn_id: row.get(3)?,
        schema_version,
        object_id: ObjectId(row.get(5)?),
        digest: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn collect_json_strings(value: &serde_json::Value, key: &str, output: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(value) = object.get(key).and_then(serde_json::Value::as_str) {
                output.push(value.to_string());
            }
            for value in object.values() {
                collect_json_strings(value, key, output);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_json_strings(value, key, output);
            }
        }
        _ => {}
    }
}

fn sort_dedup_coverage(coverage: &mut TurnEvidenceCoverage) {
    coverage.receipt_ids.sort();
    coverage.receipt_ids.dedup();
    coverage.event_ids.sort();
    coverage.event_ids.dedup();
    coverage.message_ids.sort();
    coverage.message_ids.dedup();
    coverage.artifact_ids.sort();
    coverage.artifact_ids.dedup();
    coverage
        .change_ids
        .sort_by(|left, right| left.0.cmp(&right.0));
    coverage.change_ids.dedup();
    coverage.tool_span_ids.sort();
    coverage.tool_span_ids.dedup();
    coverage.approval_ids.sort();
    coverage.approval_ids.dedup();
}

fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(Error::from)
}

fn digest_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex::encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_turn_evidence_manifest_is_exact_content_addressed_and_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        db.spawn_lane("evidence", None, false, Some("codex".to_string()), None)
            .unwrap();
        let session = db
            .start_lane_session("evidence", Some("evidence".to_string()), None)
            .unwrap()
            .session;
        let turn = db
            .begin_lane_session_turn("evidence", &session.session_id, None)
            .unwrap()
            .turn;
        let message = db
            .add_lane_turn_message(&turn.turn_id, "user", "make it exact")
            .unwrap();
        db.add_lane_turn_event(
            &turn.turn_id,
            "tool.completed",
            Some(serde_json::json!({
                "event_id":"normalized-event-1",
                "evidence":{"receipt_id":"receipt-1"},
                "span_id":"span-1"
            })),
            None,
            Some(&message.message_id.0),
        )
        .unwrap();
        let artifact = db
            .record_lane_artifact(LaneArtifactInput {
                lane: "evidence".to_string(),
                session_id: session.session_id.clone(),
                turn_id: Some(turn.turn_id.clone()),
                provider: "codex".to_string(),
                artifact_kind: "native_transcript".to_string(),
                format: "jsonl".to_string(),
                source: AgentEvidenceSource::NativeTranscript,
                source_locator_redacted: None,
                content: b"transcript".to_vec(),
                start_offset: Some(0),
                end_offset: Some(10),
                redaction_profile: None,
                trust: "provider-native".to_string(),
                supersedes_artifact_id: None,
                metadata_json: None,
            })
            .unwrap();
        db.end_lane_turn(&turn.turn_id, "completed").unwrap();

        let first = db.create_turn_evidence_manifest(&turn.turn_id).unwrap();
        let second = db.create_turn_evidence_manifest(&turn.turn_id).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.statement.coverage.receipt_ids, vec!["receipt-1"]);
        assert!(first
            .statement
            .coverage
            .event_ids
            .contains(&"normalized-event-1".to_string()));
        assert_eq!(
            first.statement.coverage.message_ids,
            vec![message.message_id.0.clone()]
        );
        assert_eq!(
            first.statement.coverage.artifact_ids,
            vec![artifact.artifact_id.clone()]
        );
        assert_eq!(first.statement.coverage.tool_span_ids, vec!["span-1"]);
        assert!(first.digest.starts_with("sha256:"));

        let prompt_node_input = ProvenanceNodeInput {
            session_id: session.session_id.clone(),
            turn_id: Some(turn.turn_id.clone()),
            node_kind: "user_message".to_string(),
            summary: "user requested exact evidence".to_string(),
            event_id: None,
            span_id: None,
            message_id: Some(message.message_id.0.clone()),
            change_id: None,
            artifact_id: None,
            source_confidence: "native-structured".to_string(),
            classifier_version: None,
            attributes: Some(serde_json::json!({"role":"user"})),
        };
        let prompt_node = db
            .create_provenance_node(prompt_node_input.clone())
            .unwrap();
        assert_eq!(
            db.create_provenance_node(prompt_node_input)
                .unwrap()
                .provenance_node_id,
            prompt_node.provenance_node_id
        );
        let tool_node = db
            .create_provenance_node(ProvenanceNodeInput {
                session_id: session.session_id.clone(),
                turn_id: Some(turn.turn_id.clone()),
                node_kind: "tool_result".to_string(),
                summary: "tool completed".to_string(),
                event_id: Some("normalized-event-1".to_string()),
                span_id: Some("span-1".to_string()),
                message_id: None,
                change_id: None,
                artifact_id: Some(artifact.artifact_id),
                source_confidence: "native-structured".to_string(),
                classifier_version: None,
                attributes: None,
            })
            .unwrap();
        let edge_input = ProvenanceEdgeInput {
            from_node_id: prompt_node.provenance_node_id.clone(),
            to_node_id: tool_node.provenance_node_id.clone(),
            relation: "caused".to_string(),
            source_confidence: "observed-order".to_string(),
            receipt_id: None,
            attributes: None,
        };
        let edge = db.create_provenance_edge(edge_input.clone()).unwrap();
        assert_eq!(
            db.create_provenance_edge(edge_input)
                .unwrap()
                .provenance_edge_id,
            edge.provenance_edge_id
        );
        let (nodes, edges) = db
            .list_session_provenance(&session.session_id, 100)
            .unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(edges, vec![edge]);

        let classification = db
            .classify_session_activity(&session.session_id, 100)
            .unwrap();
        assert_eq!(classification.classifier_version, "trail-activity-rules/v1");
        assert_eq!(classification.nodes.len(), 2);
        assert_eq!(classification.edges.len(), 1);
        assert!(classification
            .nodes
            .iter()
            .any(|node| node.node_kind == "tool_activity"
                && node.source_confidence == "deterministic-derived"));
        assert_eq!(
            db.classify_session_activity(&session.session_id, 100)
                .unwrap()
                .edges[0]
                .provenance_edge_id,
            classification.edges[0].provenance_edge_id
        );

        let first_attestation = db
            .create_session_attestation(
                &session.session_id,
                "on-end",
                Some(serde_json::json!({"test":true})),
            )
            .unwrap();
        assert_eq!(first_attestation.turns.len(), 1);
        assert_eq!(
            db.create_session_attestation(&session.session_id, "on-end", None)
                .unwrap()
                .attestation_id,
            first_attestation.attestation_id
        );
        assert!(
            db.verify_session_attestation(&first_attestation.attestation_id)
                .unwrap()
                .valid
        );

        let second_turn = db
            .begin_lane_session_turn("evidence", &session.session_id, None)
            .unwrap()
            .turn;
        db.add_lane_turn_message(&second_turn.turn_id, "user", "second turn")
            .unwrap();
        db.end_lane_turn(&second_turn.turn_id, "completed").unwrap();
        let second_attestation = db
            .create_session_attestation(&session.session_id, "on-end", None)
            .unwrap();
        assert_eq!(
            second_attestation.previous_attestation_id.as_deref(),
            Some(first_attestation.attestation_id.as_str())
        );
        assert_eq!(second_attestation.turns.len(), 1);
        assert_eq!(
            second_attestation.turns[0].turn_id,
            second_turn.turn_id.clone()
        );
        assert!(
            db.verify_session_attestation(&second_attestation.attestation_id)
                .unwrap()
                .valid
        );

        let learning = db
            .propose_learning(LearningInput {
                session_id: session.session_id.clone(),
                turn_id: Some(second_turn.turn_id.clone()),
                scope: "workspace".to_string(),
                body: "api_token=secret".to_string(),
                confidence: Some(0.8),
                source_artifact_id: None,
                anchor: Some(serde_json::json!({"turn":second_turn.turn_id})),
                expires_at: None,
                metadata: None,
            })
            .unwrap();
        assert_eq!(learning.body, "api_token=[REDACTED]");
        let accepted = db
            .review_learning(&learning.learning_id, true, "reviewer@example")
            .unwrap();
        assert_eq!(accepted.status, "accepted");
        assert_eq!(
            db.list_learnings(Some(&session.session_id), Some("accepted"), 10)
                .unwrap(),
            vec![accepted.clone()]
        );
        assert_eq!(
            db.accepted_learning_context("evidence", &["workspace".to_string()], 1_024)
                .unwrap(),
            vec![accepted.clone()]
        );
        let replacement = db
            .propose_learning(LearningInput {
                body: "use the verified replacement".to_string(),
                confidence: Some(0.9),
                anchor: Some(serde_json::json!({"supersedes":learning.learning_id})),
                ..LearningInput {
                    session_id: session.session_id.clone(),
                    turn_id: Some(second_turn.turn_id.clone()),
                    scope: "workspace".to_string(),
                    body: String::new(),
                    confidence: None,
                    source_artifact_id: None,
                    anchor: None,
                    expires_at: None,
                    metadata: None,
                }
            })
            .unwrap();
        let replacement = db
            .review_learning(&replacement.learning_id, true, "reviewer@example")
            .unwrap();
        assert_eq!(
            db.supersede_learning(
                &learning.learning_id,
                &replacement.learning_id,
                "reviewer@example"
            )
            .unwrap()
            .status,
            "superseded"
        );

        let link_input = GitAgentLinkInput {
            session_id: session.session_id.clone(),
            turn_id: Some(second_turn.turn_id.clone()),
            git_commit: "a".repeat(40),
            from_change: Some(second_turn.before_change.0.clone()),
            through_change: Some(second_turn.before_change.0.clone()),
            confidence: "exact-change".to_string(),
            source: "trail-export".to_string(),
            metadata: None,
        };
        let link = db.link_git_commit_to_agent(link_input.clone()).unwrap();
        assert_eq!(
            db.link_git_commit_to_agent(link_input)
                .unwrap()
                .git_agent_link_id,
            link.git_agent_link_id
        );
        assert_eq!(
            db.list_git_agent_links(&session.session_id).unwrap(),
            vec![link]
        );

        let secret = [7_u8; 32];
        let signed = db
            .sign_session_attestation(&second_attestation.attestation_id, &secret)
            .unwrap();
        assert_eq!(signed.status, "signed");
        let signed_verification = db
            .verify_session_attestation(&signed.attestation_id)
            .unwrap();
        assert!(signed_verification.valid);
        assert_eq!(signed_verification.signature_status, "valid");
        let public = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
        let revocation = db
            .revoke_attestation_key(&public, "test rotation", None)
            .unwrap();
        assert!(revocation.key_id.starts_with("ed25519_"));
        let revoked_verification = db
            .verify_session_attestation(&signed.attestation_id)
            .unwrap();
        assert!(!revoked_verification.valid);
        assert_eq!(revoked_verification.signature_status, "revoked");

        let trace = db.export_agent_trace(&session.session_id, true).unwrap();
        assert!(trace.verify().valid);
        let bytes = trace.to_canonical_json().unwrap();
        let imported = PortableAgentTrace::from_json(&bytes).unwrap();
        assert_eq!(imported.to_canonical_json().unwrap(), bytes);
        let mut tampered = imported;
        tampered.artifacts[0].content.as_mut().unwrap()[0] ^= 1;
        assert!(!tampered.verify().artifact_digests_valid);

        let manifest_digest = first.digest.clone();
        let artifact_id = trace.artifacts[0].artifact_id.clone();
        let redacted = db
            .redact_lane_artifact(&artifact_id, "retention policy")
            .unwrap();
        assert_eq!(redacted.retention_status, "redacted");
        assert!(redacted.content_object_id.is_none());
        assert!(db.lane_artifact_content(&artifact_id).is_err());
        assert_eq!(
            db.turn_evidence_manifest(&turn.turn_id).unwrap().digest,
            manifest_digest
        );
    }

    #[test]
    fn open_turn_cannot_freeze_an_evidence_manifest() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        db.spawn_lane("open-evidence", None, false, None, None)
            .unwrap();
        let turn = db
            .begin_lane_turn("open-evidence", None, None, None)
            .unwrap()
            .turn;
        assert!(db.create_turn_evidence_manifest(&turn.turn_id).is_err());
    }
}
