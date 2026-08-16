//! Durable workspace behaviour: creation and optimistic concurrency, revision
//! history, template application and export snapshots, and the client
//! delete/restore lifecycle. Nothing here drives a Bedrock conversation — the
//! turn loop's own spec lives in `claria-report-pipeline`.

use std::collections::HashSet;

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use claria_core::models::report::{
    ReportBlock, ReportContent, ReportSection, ReportTemplateWarning, ReportTemplateWarningCode,
};
use claria_mock_aws::testing::MockServer;
use claria_report_store::{self as store, REPORT_CONFLICT_MESSAGE};
use claria_storage::client;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const BUCKET: &str = "claria-report-store-test";

fn sdk_config(endpoint: &str) -> aws_config::SdkConfig {
    let credentials = Credentials::new("test", "test", None, None, "test");
    aws_config::SdkConfig::builder()
        .region(aws_config::Region::new("us-east-1"))
        .credentials_provider(SharedCredentialsProvider::new(credentials))
        .endpoint_url(endpoint)
        .behavior_version(aws_config::BehaviorVersion::latest())
        .build()
}

async fn setup() -> (MockServer, aws_sdk_s3::Client) {
    let server = MockServer::spawn().await;
    let s3 = client::from_config(&sdk_config(&server.endpoint));
    s3.create_bucket()
        .bucket(BUCKET)
        .send()
        .await
        .expect("create bucket");
    (server, s3)
}

async fn put_client(s3: &aws_sdk_s3::Client, client_id: Uuid) {
    let now = jiff::Timestamp::now();
    let client = claria_core::models::client::Client {
        id: client_id,
        name: "Test client".to_string(),
        created_at: now,
        updated_at: now,
    };
    claria_storage::objects::put_object(
        s3,
        BUCKET,
        &claria_core::s3_keys::client(client_id),
        serde_json::to_vec(&client).expect("client JSON"),
        Some("application/json"),
    )
    .await
    .expect("put client");
}

fn paragraph(text: &str) -> ReportBlock {
    ReportBlock::Paragraph {
        text: text.to_string(),
    }
}

#[tokio::test]
async fn writing_sessions_start_fresh_and_resume_independently_by_report_id() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    put_client(&s3, client_id).await;

    let first = store::start_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("start first Writing session");
    let replayed = store::start_report_workspace_with_id(&s3, BUCKET, client_id, first.report_id)
        .await
        .expect("replay first start");
    assert_eq!(replayed.report_id, first.report_id);
    store::save_report_draft_for_report(
        &s3,
        BUCKET,
        client_id,
        first.report_id,
        0,
        ReportContent {
            title: "First report".to_string(),
            sections: vec![],
        },
    )
    .await
    .expect("save first report");

    let second = store::start_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("start second Writing session");
    assert_ne!(first.report_id, second.report_id);
    assert_eq!(first.session_name, "Writer Session (1)");
    assert_eq!(second.session_name, "Writer Session (2)");
    store::save_report_draft_for_report(
        &s3,
        BUCKET,
        client_id,
        second.report_id,
        0,
        ReportContent {
            title: "Second report".to_string(),
            sections: vec![],
        },
    )
    .await
    .expect("save second report");

    let sessions = store::list_report_workspaces(&s3, BUCKET, client_id)
        .await
        .expect("list Writing sessions");
    assert_eq!(sessions.len(), 2);
    let resumed_first = store::load_report_workspace_by_id(&s3, BUCKET, client_id, first.report_id)
        .await
        .expect("resume first Writing session");
    let resumed_second =
        store::load_report_workspace_by_id(&s3, BUCKET, client_id, second.report_id)
            .await
            .expect("resume second Writing session");
    assert_eq!(resumed_first.draft.content.title, "First report");
    assert_eq!(resumed_second.draft.content.title, "Second report");
}

#[tokio::test]
async fn legacy_singleton_remains_resumable_beside_new_writing_sessions() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    put_client(&s3, client_id).await;
    let legacy = store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("legacy workspace");
    store::save_report_draft(
        &s3,
        BUCKET,
        client_id,
        0,
        ReportContent {
            title: "Legacy report".to_string(),
            sections: vec![],
        },
    )
    .await
    .expect("save legacy report");

    let fresh = store::start_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("new session");
    assert_eq!(fresh.session_name, "Writer Session (2)");
    let listed = store::list_report_workspaces(&s3, BUCKET, client_id)
        .await
        .expect("list mixed sessions");
    assert_eq!(listed.len(), 2);
    let resumed = store::load_report_workspace_by_id(&s3, BUCKET, client_id, legacy.report_id)
        .await
        .expect("resume legacy session");
    assert_eq!(resumed.draft.content.title, "Legacy report");
}

#[tokio::test]
async fn lazy_workspace_and_manual_edits_are_versioned_and_conflict_safe() {
    let (_server, s3) = setup().await;
    let client_id = Uuid::new_v4();
    put_client(&s3, client_id).await;

    assert!(
        store::find_report_workspace(&s3, BUCKET, client_id)
            .await
            .expect("look up absent workspace")
            .is_none()
    );
    assert!(
        store::find_report_workspace(&s3, BUCKET, client_id)
            .await
            .expect("repeat non-creating lookup")
            .is_none()
    );
    let first = store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("create workspace");
    let reloaded = store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("reload workspace");
    assert_eq!(reloaded.report_id, first.report_id);
    assert_eq!(reloaded.draft.revision, 0);

    let saved = store::save_report_draft(
        &s3,
        BUCKET,
        client_id,
        0,
        ReportContent {
            title: "Manual report".to_string(),
            sections: vec![
                ReportSection {
                    skipped: false,
                    id: Uuid::new_v4(),
                    heading: "First".to_string(),
                    blocks: vec![paragraph("One")],
                },
                ReportSection {
                    skipped: false,
                    id: Uuid::new_v4(),
                    heading: "Second".to_string(),
                    blocks: vec![paragraph("Two")],
                },
            ],
        },
    )
    .await
    .expect("save");
    assert_eq!(saved.draft.revision, 1);
    assert_eq!(
        store::find_report_workspace(&s3, BUCKET, client_id)
            .await
            .expect("find writing history")
            .expect("persisted session")
            .report_id,
        first.report_id
    );
    let exported = store::record_report_export(
        &s3,
        BUCKET,
        client_id,
        first.report_id,
        1,
        claria_core::models::report::ReportExportStatus::Exported,
    )
    .await
    .expect("record export");
    assert_eq!(
        exported
            .session
            .last_export
            .as_ref()
            .map(|export| export.status),
        Some(claria_core::models::report::ReportExportStatus::Exported)
    );
    let first_id = saved.draft.content.sections[0].id;
    let second_id = saved.draft.content.sections[1].id;
    assert_ne!(first_id, second_id);

    let reordered = store::save_report_draft(
        &s3,
        BUCKET,
        client_id,
        1,
        ReportContent {
            title: "Manual report".to_string(),
            sections: vec![
                ReportSection {
                    skipped: false,
                    id: second_id,
                    heading: "Second".to_string(),
                    blocks: vec![paragraph("Two")],
                },
                ReportSection {
                    skipped: false,
                    id: first_id,
                    heading: "First".to_string(),
                    blocks: vec![paragraph("One")],
                },
            ],
        },
    )
    .await
    .expect("reorder");
    assert_eq!(reordered.draft.revision, 2);
    assert_eq!(reordered.draft.content.sections[0].id, second_id);

    let stale = store::save_report_draft(
        &s3,
        BUCKET,
        client_id,
        1,
        ReportContent {
            title: "Stale overwrite".to_string(),
            sections: vec![],
        },
    )
    .await
    .unwrap_err();
    assert_eq!(stale.to_string(), REPORT_CONFLICT_MESSAGE);

    let final_state = store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("reload");
    assert_eq!(final_state.draft.revision, 2);
    assert_eq!(final_state.draft.content.title, "Manual report");
}

#[tokio::test]
async fn report_revisions_can_be_previewed_and_restored_without_removing_history() {
    let (_server, s3) = setup().await;
    s3.put_bucket_versioning()
        .bucket(BUCKET)
        .versioning_configuration(
            aws_sdk_s3::types::VersioningConfiguration::builder()
                .status(aws_sdk_s3::types::BucketVersioningStatus::Enabled)
                .build(),
        )
        .send()
        .await
        .expect("enable versioning");
    let client_id = Uuid::new_v4();
    put_client(&s3, client_id).await;
    let initial = store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("workspace");

    let first = store::save_report_draft(
        &s3,
        BUCKET,
        client_id,
        0,
        ReportContent {
            title: "First version".to_string(),
            sections: vec![],
        },
    )
    .await
    .expect("save first version");
    let second = store::save_report_draft(
        &s3,
        BUCKET,
        client_id,
        1,
        ReportContent {
            title: "Second version".to_string(),
            sections: vec![],
        },
    )
    .await
    .expect("save second version");
    store::rename_report_session(&s3, BUCKET, client_id, initial.report_id, "Renamed session")
        .await
        .expect("save another object version at the same draft revision");

    let cache = store::RevisionCache::new();
    let revisions = store::list_report_revisions(&s3, BUCKET, client_id, initial.report_id, &cache)
        .await
        .expect("list revisions");
    assert_eq!(
        revisions
            .iter()
            .map(|revision| revision.revision)
            .collect::<Vec<_>>(),
        vec![2, 1, 0]
    );
    assert_eq!(revisions[0].title, "Second version");

    let historical = store::load_report_revision(
        &s3,
        BUCKET,
        client_id,
        initial.report_id,
        first.draft.revision,
        &cache,
    )
    .await
    .expect("load first revision");
    assert_eq!(historical.content.title, "First version");

    let restored = store::revert_report_revision(
        &s3,
        BUCKET,
        client_id,
        initial.report_id,
        second.draft.revision,
        first.draft.revision,
        &cache,
    )
    .await
    .expect("restore first revision");
    assert_eq!(restored.draft.revision, 3);
    assert_eq!(restored.draft.content.title, "First version");

    let revisions_after_restore =
        store::list_report_revisions(&s3, BUCKET, client_id, initial.report_id, &cache)
            .await
            .expect("list revisions after restore");
    assert_eq!(
        revisions_after_restore
            .iter()
            .map(|revision| revision.revision)
            .collect::<Vec<_>>(),
        vec![3, 2, 1, 0]
    );
    let preserved = store::load_report_revision(
        &s3,
        BUCKET,
        client_id,
        initial.report_id,
        second.draft.revision,
        &cache,
    )
    .await
    .expect("load revision that preceded restore");
    assert_eq!(preserved.content.title, "Second version");

    let current_error = store::revert_report_revision(
        &s3,
        BUCKET,
        client_id,
        initial.report_id,
        restored.draft.revision,
        restored.draft.revision,
        &cache,
    )
    .await
    .expect_err("current revision cannot be restored");
    assert!(
        current_error
            .to_string()
            .contains("earlier report revision")
    );
}

#[tokio::test]
async fn template_import_exports_without_confirmation_and_keeps_its_source_package() {
    let (_server, s3) = setup().await;
    s3.put_bucket_versioning()
        .bucket(BUCKET)
        .versioning_configuration(
            aws_sdk_s3::types::VersioningConfiguration::builder()
                .status(aws_sdk_s3::types::BucketVersioningStatus::Enabled)
                .build(),
        )
        .send()
        .await
        .expect("enable versioning");
    let client_id = Uuid::new_v4();
    put_client(&s3, client_id).await;
    let initial = store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("workspace");
    let template_source = b"validated redacted template package".to_vec();
    let source_sha256 = Sha256::digest(&template_source)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    store::store_report_template_source(
        &s3,
        BUCKET,
        client_id,
        &source_sha256,
        template_source.clone(),
    )
    .await
    .expect("store source package");
    let imported = store::apply_report_template(
        &s3,
        BUCKET,
        client_id,
        0,
        store::ReportTemplateApplication {
            content: ReportContent {
                title: "Imported template".to_string(),
                sections: vec![ReportSection {
                    skipped: false,
                    id: Uuid::new_v4(),
                    heading: "Scores".to_string(),
                    blocks: vec![ReportBlock::Table {
                        rows: vec![
                            vec!["Measure".to_string(), "Score".to_string()],
                            vec!["Attention".to_string(), "".to_string()],
                        ],
                        has_header: true,
                        column_widths: Some(vec![7_000, 3_000]),
                    }],
                }],
            },
            source_sha256,
            writer_template_id: Uuid::new_v4(),
            writer_template_name: "Clinical report".to_string(),
            warnings: vec![ReportTemplateWarning {
                code: ReportTemplateWarningCode::HeadersFootersOmitted,
                count: 2,
            }],
        },
    )
    .await
    .expect("apply template");
    assert_eq!(imported.draft.revision, 1);
    let metadata = imported
        .template_import
        .as_ref()
        .expect("template metadata");
    assert_eq!(metadata.imported_revision, 1);
    assert_eq!(metadata.reviewed_revision, Some(1));
    assert_eq!(
        metadata.writer_template_name.as_deref(),
        Some("Clinical report")
    );

    let snapshot = store::load_export_snapshot(&s3, BUCKET, client_id, initial.report_id, 1)
        .await
        .expect("export without confirmation");
    assert_eq!(snapshot.draft.revision, 1);
    assert_eq!(snapshot.template_source, Some(template_source.clone()));

    let edited = store::save_report_draft(
        &s3,
        BUCKET,
        client_id,
        1,
        ReportContent {
            title: "Imported template revised".to_string(),
            sections: imported.draft.content.sections,
        },
    )
    .await
    .expect("edit imported report");
    assert_eq!(edited.draft.revision, 2);
    assert_eq!(
        edited
            .template_import
            .as_ref()
            .and_then(|template| template.reviewed_revision),
        Some(2)
    );
    assert_eq!(
        store::load_export_snapshot(&s3, BUCKET, client_id, initial.report_id, 2)
            .await
            .expect("edited template export")
            .draft
            .revision,
        2
    );

    let second_template = store::apply_report_template(
        &s3,
        BUCKET,
        client_id,
        2,
        store::ReportTemplateApplication {
            content: ReportContent {
                title: "Different template".to_string(),
                sections: vec![],
            },
            source_sha256: "c".repeat(64),
            writer_template_id: Uuid::new_v4(),
            writer_template_name: "Different template".to_string(),
            warnings: vec![],
        },
    )
    .await
    .expect_err("a session cannot switch templates");
    assert!(
        second_template
            .to_string()
            .contains("already has a template")
    );

    let persisted = claria_storage::objects::get_object(
        &s3,
        BUCKET,
        &claria_core::s3_keys::report_workspace(client_id),
    )
    .await
    .expect("workspace object");
    let json = String::from_utf8(persisted.body).expect("workspace JSON");
    assert!(!json.contains("local_path"));
    assert!(!json.contains("source_filename"));

    store::delete_report_workspace_for_client(&s3, BUCKET, client_id)
        .await
        .expect("delete report objects");
    assert!(
        store::restore_report_workspace_for_client(&s3, BUCKET, client_id)
            .await
            .expect("restore report")
    );
    let restored = store::load_export_snapshot(&s3, BUCKET, client_id, initial.report_id, 2)
        .await
        .expect("restored export snapshot");
    assert_eq!(restored.template_source, Some(template_source));
}

#[tokio::test]
async fn client_delete_and_restore_round_trip_the_latest_report_workspace() {
    let (server, s3) = setup().await;
    s3.put_bucket_versioning()
        .bucket(BUCKET)
        .versioning_configuration(
            aws_sdk_s3::types::VersioningConfiguration::builder()
                .status(aws_sdk_s3::types::BucketVersioningStatus::Enabled)
                .build(),
        )
        .send()
        .await
        .expect("enable versioning");
    let client_id = Uuid::new_v4();
    put_client(&s3, client_id).await;
    let created = store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("create");
    let saved = store::save_report_draft(
        &s3,
        BUCKET,
        client_id,
        0,
        ReportContent {
            title: "Restorable report".to_string(),
            sections: vec![],
        },
    )
    .await
    .expect("save");
    assert_eq!(saved.draft.revision, 1);

    // Equal timestamps must not make restoration choose an older revision.
    let key = claria_core::s3_keys::report_workspace(client_id);
    if let Some(versions) = server
        .state
        .write()
        .await
        .objects
        .get_mut(&(BUCKET.to_string(), key.clone()))
    {
        for version in versions {
            version.last_modified = "2026-08-01T12:00:00Z".to_string();
        }
    }

    assert_eq!(
        store::delete_report_workspace_for_client(&s3, BUCKET, client_id)
            .await
            .expect("delete"),
        1
    );
    assert!(matches!(
        claria_storage::objects::get_object(&s3, BUCKET, &key).await,
        Err(claria_storage::error::StorageError::NotFound { .. })
    ));

    let first_client = s3.clone();
    let second_client = s3.clone();
    let (first_restore, second_restore) = tokio::join!(
        store::restore_report_workspace_for_client(&first_client, BUCKET, client_id),
        store::restore_report_workspace_for_client(&second_client, BUCKET, client_id)
    );
    assert!(first_restore.expect("first restore"));
    assert!(second_restore.expect("second restore"));
    let restored = store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("reload");
    assert_eq!(restored.report_id, created.report_id);
    assert_eq!(restored.draft.revision, 1);
    assert_eq!(restored.draft.content.title, "Restorable report");

    let edited = store::save_report_draft(
        &s3,
        BUCKET,
        client_id,
        1,
        ReportContent {
            title: "Edited after restore".to_string(),
            sections: vec![],
        },
    )
    .await
    .expect("post-restore edit");
    assert_eq!(edited.draft.revision, 2);
    assert!(
        store::restore_report_workspace_for_client(&s3, BUCKET, client_id)
            .await
            .expect("retried restore")
    );
    let after_retry = store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("load after retry");
    assert_eq!(after_retry.draft.revision, 2);
    assert_eq!(after_retry.draft.content.title, "Edited after restore");
}

#[tokio::test]
async fn client_lifecycle_restores_every_independent_writing_session() {
    let (_server, s3) = setup().await;
    s3.put_bucket_versioning()
        .bucket(BUCKET)
        .versioning_configuration(
            aws_sdk_s3::types::VersioningConfiguration::builder()
                .status(aws_sdk_s3::types::BucketVersioningStatus::Enabled)
                .build(),
        )
        .send()
        .await
        .expect("enable versioning");
    let client_id = Uuid::new_v4();
    put_client(&s3, client_id).await;
    let first = store::start_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("first session");
    let second = store::start_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("second session");
    for (session, title) in [(&first, "First report"), (&second, "Second report")] {
        store::save_report_draft_for_report(
            &s3,
            BUCKET,
            client_id,
            session.report_id,
            0,
            ReportContent {
                title: title.to_string(),
                sections: vec![],
            },
        )
        .await
        .expect("save session");
    }

    assert_eq!(
        store::delete_report_workspace_for_client(&s3, BUCKET, client_id)
            .await
            .expect("delete sessions"),
        2
    );
    assert!(
        store::list_report_workspaces(&s3, BUCKET, client_id)
            .await
            .expect("deleted list")
            .is_empty()
    );
    assert!(
        store::restore_report_workspace_for_client(&s3, BUCKET, client_id)
            .await
            .expect("restore sessions")
    );
    let restored = store::list_report_workspaces(&s3, BUCKET, client_id)
        .await
        .expect("restored list");
    assert_eq!(restored.len(), 2);
    let titles = restored
        .iter()
        .map(|workspace| workspace.draft.content.title.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(titles, HashSet::from(["First report", "Second report"]));
}

#[tokio::test]
async fn workspace_creation_requires_a_current_client_and_export_is_revision_bound() {
    let (_server, s3) = setup().await;
    let missing_client = Uuid::new_v4();
    let error = store::load_report_workspace(&s3, BUCKET, missing_client)
        .await
        .expect_err("orphan workspace");
    assert!(matches!(error, store::ReportStoreError::ClientNotFound));
    assert!(matches!(
        claria_storage::objects::get_object(
            &s3,
            BUCKET,
            &claria_core::s3_keys::report_workspace(missing_client)
        )
        .await,
        Err(claria_storage::error::StorageError::NotFound { .. })
    ));

    let client_id = Uuid::new_v4();
    put_client(&s3, client_id).await;
    let initial = store::load_report_workspace(&s3, BUCKET, client_id)
        .await
        .expect("workspace");
    store::save_report_draft(
        &s3,
        BUCKET,
        client_id,
        0,
        ReportContent {
            title: "New revision".to_string(),
            sections: vec![],
        },
    )
    .await
    .expect("save");
    let error = store::load_export_snapshot(&s3, BUCKET, client_id, initial.report_id, 0)
        .await
        .expect_err("stale export revision");
    assert_eq!(error.to_string(), REPORT_CONFLICT_MESSAGE);
}

#[test]
fn exported_filename_is_sanitized_and_has_docx_extension() {
    assert_eq!(
        store::suggested_docx_filename("  Clinical / Report: July  "),
        "Clinical-Report-July.docx"
    );
    assert_eq!(store::suggested_docx_filename("///"), "report.docx");
}
