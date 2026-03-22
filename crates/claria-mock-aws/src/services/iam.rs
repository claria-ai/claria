use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{
    params,
    state::{AccessKeyRecord, IamPolicy, IamPolicyVersion, IamUser, SharedState},
    xml,
};

/// Dispatch IAM actions from form-encoded POST body.
pub async fn dispatch(action: &str, params: &str, state: SharedState) -> Response {
    match action {
        "GetUser" => get_user(params, state).await,
        "CreateUser" => create_user(params, state).await,
        "ListUsers" => list_users(state).await,
        "CreatePolicy" => create_policy(params, state).await,
        "GetPolicy" => get_policy(params, state).await,
        "GetPolicyVersion" => get_policy_version(params, state).await,
        "CreatePolicyVersion" => create_policy_version(params, state).await,
        "ListPolicyVersions" => list_policy_versions(params, state).await,
        "DeletePolicyVersion" => delete_policy_version(params, state).await,
        "AttachUserPolicy" => attach_user_policy(params, state).await,
        "DetachUserPolicy" => detach_user_policy(params, state).await,
        "ListAttachedUserPolicies" => list_attached_user_policies(params, state).await,
        "GetUserPolicy" => get_user_policy(params, state).await,
        "PutUserPolicy" => put_user_policy(params, state).await,
        "DeleteUserPolicy" => delete_user_policy(params, state).await,
        "CreateAccessKey" => create_access_key(params, state).await,
        "ListAccessKeys" => list_access_keys(params, state).await,
        "DeleteAccessKey" => delete_access_key(params, state).await,
        "GetAccessKeyLastUsed" => get_access_key_last_used(params, state).await,
        _ => (
            StatusCode::BAD_REQUEST,
            xml::error_xml("InvalidAction", &format!("Unknown IAM action: {action}")),
        )
            .into_response(),
    }
}

fn param(p: &str, key: &str) -> Option<String> {
    params::extract(p, key)
}

fn xml_response(body: String) -> Response {
    (StatusCode::OK, [("content-type", "text/xml")], body).into_response()
}

// ── User operations ──

async fn get_user(params: &str, state: SharedState) -> Response {
    let user_name = param(params, "UserName").unwrap_or_default();
    let st = state.read().await;
    match st.users.get(&user_name) {
        Some(user) => xml_response(xml::xml_doc(&xml::wrap(
            "GetUserResponse",
            &xml::wrap(
                "GetUserResult",
                &xml::wrap(
                    "User",
                    &format!(
                        "{}{}{}{}",
                        xml::el("UserName", &user.user_name),
                        xml::el("Arn", &user.arn),
                        xml::el("UserId", &user.user_id),
                        xml::el("CreateDate", &user.create_date),
                    ),
                ),
            ),
        ))),
        None => (
            StatusCode::NOT_FOUND,
            xml::error_xml("NoSuchEntity", &format!("User {user_name} not found")),
        )
            .into_response(),
    }
}

async fn create_user(params: &str, state: SharedState) -> Response {
    let user_name = param(params, "UserName").unwrap_or_default();
    let mut st = state.write().await;
    let account = &st.caller_identity.account.clone();

    if st.users.contains_key(&user_name) {
        return (
            StatusCode::CONFLICT,
            xml::error_xml("EntityAlreadyExists", "User already exists"),
        )
            .into_response();
    }

    let user = IamUser {
        arn: format!("arn:aws:iam::{account}:user/{user_name}"),
        user_id: format!("AIDA{}", &Uuid::new_v4().to_string()[..16].to_uppercase()),
        user_name: user_name.clone(),
        create_date: jiff::Timestamp::now().to_string(),
    };

    let arn = user.arn.clone();
    st.users.insert(user_name, user);

    xml_response(xml::xml_doc(&xml::wrap(
        "CreateUserResponse",
        &xml::wrap(
            "CreateUserResult",
            &xml::wrap("User", &xml::el("Arn", &arn)),
        ),
    )))
}

async fn list_users(state: SharedState) -> Response {
    let st = state.read().await;
    let mut members = String::new();
    for user in st.users.values() {
        members.push_str(&xml::wrap(
            "member",
            &format!(
                "{}{}{}",
                xml::el("UserName", &user.user_name),
                xml::el("Arn", &user.arn),
                xml::el("UserId", &user.user_id),
            ),
        ));
    }

    xml_response(xml::xml_doc(&xml::wrap(
        "ListUsersResponse",
        &xml::wrap(
            "ListUsersResult",
            &xml::wrap("Users", &members),
        ),
    )))
}

// ── Policy operations ──

async fn create_policy(params: &str, state: SharedState) -> Response {
    let policy_name = param(params, "PolicyName").unwrap_or_default();
    let document = param(params, "PolicyDocument").unwrap_or_default();
    let description = param(params, "Description").unwrap_or_default();
    let mut st = state.write().await;
    let account = st.caller_identity.account.clone();
    let arn = format!("arn:aws:iam::{account}:policy/{policy_name}");

    if st.policies.contains_key(&arn) {
        return (
            StatusCode::CONFLICT,
            xml::error_xml("EntityAlreadyExists", "Policy already exists"),
        )
            .into_response();
    }

    let version = IamPolicyVersion {
        version_id: "v1".to_string(),
        document,
        is_default: true,
        create_date: jiff::Timestamp::now().to_string(),
    };

    let policy = IamPolicy {
        arn: arn.clone(),
        policy_name: policy_name.clone(),
        description,
        default_version_id: "v1".to_string(),
        versions: vec![version],
    };

    st.policies.insert(arn.clone(), policy);

    xml_response(xml::xml_doc(&xml::wrap(
        "CreatePolicyResponse",
        &xml::wrap(
            "CreatePolicyResult",
            &xml::wrap(
                "Policy",
                &format!(
                    "{}{}",
                    xml::el("Arn", &arn),
                    xml::el("DefaultVersionId", "v1"),
                ),
            ),
        ),
    )))
}

async fn get_policy(params: &str, state: SharedState) -> Response {
    let arn = param(params, "PolicyArn").unwrap_or_default();
    let st = state.read().await;
    match st.policies.get(&arn) {
        Some(policy) => xml_response(xml::xml_doc(&xml::wrap(
            "GetPolicyResponse",
            &xml::wrap(
                "GetPolicyResult",
                &xml::wrap(
                    "Policy",
                    &format!(
                        "{}{}{}",
                        xml::el("Arn", &policy.arn),
                        xml::el("PolicyName", &policy.policy_name),
                        xml::el("DefaultVersionId", &policy.default_version_id),
                    ),
                ),
            ),
        ))),
        None => (
            StatusCode::NOT_FOUND,
            xml::error_xml("NoSuchEntity", "Policy not found"),
        )
            .into_response(),
    }
}

async fn get_policy_version(params: &str, state: SharedState) -> Response {
    let arn = param(params, "PolicyArn").unwrap_or_default();
    let version_id = param(params, "VersionId").unwrap_or_default();
    let st = state.read().await;

    let policy = match st.policies.get(&arn) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                xml::error_xml("NoSuchEntity", "Policy not found"),
            )
                .into_response()
        }
    };

    let version = match policy.versions.iter().find(|v| v.version_id == version_id) {
        Some(v) => v,
        None => {
            return (
                StatusCode::NOT_FOUND,
                xml::error_xml("NoSuchEntity", "Version not found"),
            )
                .into_response()
        }
    };

    // URL-encode the document (AWS returns it percent-encoded)
    let encoded_doc = percent_encoding::utf8_percent_encode(
        &version.document,
        percent_encoding::NON_ALPHANUMERIC,
    )
    .to_string();

    xml_response(xml::xml_doc(&xml::wrap(
        "GetPolicyVersionResponse",
        &xml::wrap(
            "GetPolicyVersionResult",
            &xml::wrap(
                "PolicyVersion",
                &format!(
                    "{}{}{}",
                    xml::el("VersionId", &version.version_id),
                    xml::el("Document", &encoded_doc),
                    xml::el("IsDefaultVersion", &version.is_default.to_string()),
                ),
            ),
        ),
    )))
}

async fn create_policy_version(params: &str, state: SharedState) -> Response {
    let arn = param(params, "PolicyArn").unwrap_or_default();
    let document = param(params, "PolicyDocument").unwrap_or_default();
    let set_as_default = param(params, "SetAsDefault")
        .map(|v| v == "true")
        .unwrap_or(false);

    let mut st = state.write().await;
    let policy = match st.policies.get_mut(&arn) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                xml::error_xml("NoSuchEntity", "Policy not found"),
            )
                .into_response()
        }
    };

    // AWS limit: 5 versions max
    if policy.versions.len() >= 5 {
        return (
            StatusCode::CONFLICT,
            xml::error_xml("LimitExceeded", "A managed policy can have up to 5 versions"),
        )
            .into_response();
    }

    let version_num = policy.versions.len() + 1;
    let version_id = format!("v{version_num}");

    if set_as_default {
        for v in &mut policy.versions {
            v.is_default = false;
        }
        policy.default_version_id = version_id.clone();
    }

    policy.versions.push(IamPolicyVersion {
        version_id: version_id.clone(),
        document,
        is_default: set_as_default,
        create_date: jiff::Timestamp::now().to_string(),
    });

    xml_response(xml::xml_doc(&xml::wrap(
        "CreatePolicyVersionResponse",
        &xml::wrap(
            "CreatePolicyVersionResult",
            &xml::wrap(
                "PolicyVersion",
                &xml::el("VersionId", &version_id),
            ),
        ),
    )))
}

async fn list_policy_versions(params: &str, state: SharedState) -> Response {
    let arn = param(params, "PolicyArn").unwrap_or_default();
    let st = state.read().await;
    let policy = match st.policies.get(&arn) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                xml::error_xml("NoSuchEntity", "Policy not found"),
            )
                .into_response()
        }
    };

    let mut members = String::new();
    for v in &policy.versions {
        members.push_str(&xml::wrap(
            "member",
            &format!(
                "{}{}{}",
                xml::el("VersionId", &v.version_id),
                xml::el("IsDefaultVersion", &v.is_default.to_string()),
                xml::el("CreateDate", &v.create_date),
            ),
        ));
    }

    xml_response(xml::xml_doc(&xml::wrap(
        "ListPolicyVersionsResponse",
        &xml::wrap(
            "ListPolicyVersionsResult",
            &xml::wrap("Versions", &members),
        ),
    )))
}

async fn delete_policy_version(params: &str, state: SharedState) -> Response {
    let arn = param(params, "PolicyArn").unwrap_or_default();
    let version_id = param(params, "VersionId").unwrap_or_default();
    let mut st = state.write().await;
    if let Some(policy) = st.policies.get_mut(&arn) {
        policy.versions.retain(|v| v.version_id != version_id);
    }
    xml_response(xml::xml_doc(&xml::wrap("DeletePolicyVersionResponse", "")))
}

// ── User-policy attachment ──

async fn attach_user_policy(params: &str, state: SharedState) -> Response {
    let user_name = param(params, "UserName").unwrap_or_default();
    let policy_arn = param(params, "PolicyArn").unwrap_or_default();
    let mut st = state.write().await;
    st.user_attached_policies
        .entry(user_name)
        .or_default()
        .push(policy_arn);
    xml_response(xml::xml_doc(&xml::wrap("AttachUserPolicyResponse", "")))
}

async fn detach_user_policy(params: &str, state: SharedState) -> Response {
    let user_name = param(params, "UserName").unwrap_or_default();
    let policy_arn = param(params, "PolicyArn").unwrap_or_default();
    let mut st = state.write().await;
    if let Some(policies) = st.user_attached_policies.get_mut(&user_name) {
        policies.retain(|a| a != &policy_arn);
    }
    xml_response(xml::xml_doc(&xml::wrap("DetachUserPolicyResponse", "")))
}

async fn list_attached_user_policies(params: &str, state: SharedState) -> Response {
    let user_name = param(params, "UserName").unwrap_or_default();
    let st = state.read().await;
    let policies = st.user_attached_policies.get(&user_name);

    let mut members = String::new();
    if let Some(arns) = policies {
        for arn in arns {
            let name = st
                .policies
                .get(arn)
                .map(|p| p.policy_name.as_str())
                .unwrap_or("Unknown");
            members.push_str(&xml::wrap(
                "member",
                &format!(
                    "{}{}",
                    xml::el("PolicyName", name),
                    xml::el("PolicyArn", arn),
                ),
            ));
        }
    }

    xml_response(xml::xml_doc(&xml::wrap(
        "ListAttachedUserPoliciesResponse",
        &xml::wrap(
            "ListAttachedUserPoliciesResult",
            &xml::wrap("AttachedPolicies", &members),
        ),
    )))
}

// ── Inline user policies ──

async fn get_user_policy(params: &str, state: SharedState) -> Response {
    let user_name = param(params, "UserName").unwrap_or_default();
    let policy_name = param(params, "PolicyName").unwrap_or_default();
    let st = state.read().await;

    match st
        .user_inline_policies
        .get(&(user_name.clone(), policy_name.clone()))
    {
        Some(doc) => {
            let encoded = percent_encoding::utf8_percent_encode(
                doc,
                percent_encoding::NON_ALPHANUMERIC,
            )
            .to_string();
            xml_response(xml::xml_doc(&xml::wrap(
                "GetUserPolicyResponse",
                &xml::wrap(
                    "GetUserPolicyResult",
                    &format!(
                        "{}{}{}",
                        xml::el("UserName", &user_name),
                        xml::el("PolicyName", &policy_name),
                        xml::el("PolicyDocument", &encoded),
                    ),
                ),
            )))
        }
        None => (
            StatusCode::NOT_FOUND,
            xml::error_xml("NoSuchEntity", "Policy not found"),
        )
            .into_response(),
    }
}

async fn put_user_policy(params: &str, state: SharedState) -> Response {
    let user_name = param(params, "UserName").unwrap_or_default();
    let policy_name = param(params, "PolicyName").unwrap_or_default();
    let document = param(params, "PolicyDocument").unwrap_or_default();
    let mut st = state.write().await;
    st.user_inline_policies
        .insert((user_name, policy_name), document);
    xml_response(xml::xml_doc(&xml::wrap("PutUserPolicyResponse", "")))
}

async fn delete_user_policy(params: &str, state: SharedState) -> Response {
    let user_name = param(params, "UserName").unwrap_or_default();
    let policy_name = param(params, "PolicyName").unwrap_or_default();
    let mut st = state.write().await;
    st.user_inline_policies
        .remove(&(user_name, policy_name));
    xml_response(xml::xml_doc(&xml::wrap("DeleteUserPolicyResponse", "")))
}

// ── Access keys ──

async fn create_access_key(params: &str, state: SharedState) -> Response {
    let user_name = param(params, "UserName").unwrap_or_default();
    let mut st = state.write().await;

    // Check 2-key limit
    let existing = st
        .access_keys
        .values()
        .filter(|k| k.user_name == user_name)
        .count();
    if existing >= 2 {
        return (
            StatusCode::CONFLICT,
            xml::error_xml(
                "LimitExceeded",
                "Cannot exceed quota for AccessKeysPerUser: 2",
            ),
        )
            .into_response();
    }

    let access_key_id = format!(
        "AKIA{}",
        &Uuid::new_v4().to_string().replace('-', "")[..16].to_uppercase()
    );
    let secret = Uuid::new_v4().to_string();
    let now = jiff::Timestamp::now().to_string();

    let record = AccessKeyRecord {
        access_key_id: access_key_id.clone(),
        secret_access_key: secret.clone(),
        user_name: user_name.clone(),
        status: "Active".to_string(),
        create_date: now,
        last_used_date: None,
        last_used_service: None,
    };

    st.access_keys.insert(access_key_id.clone(), record);

    xml_response(xml::xml_doc(&xml::wrap(
        "CreateAccessKeyResponse",
        &xml::wrap(
            "CreateAccessKeyResult",
            &xml::wrap(
                "AccessKey",
                &format!(
                    "{}{}{}{}",
                    xml::el("UserName", &user_name),
                    xml::el("AccessKeyId", &access_key_id),
                    xml::el("SecretAccessKey", &secret),
                    xml::el("Status", "Active"),
                ),
            ),
        ),
    )))
}

async fn list_access_keys(params: &str, state: SharedState) -> Response {
    let user_name = param(params, "UserName").unwrap_or_default();
    let st = state.read().await;

    let mut members = String::new();
    for key in st.access_keys.values() {
        if key.user_name == user_name || (user_name.is_empty() && key.user_name.is_empty()) {
            members.push_str(&xml::wrap(
                "member",
                &format!(
                    "{}{}{}",
                    xml::el("AccessKeyId", &key.access_key_id),
                    xml::el("Status", &key.status),
                    xml::el("CreateDate", &key.create_date),
                ),
            ));
        }
    }

    xml_response(xml::xml_doc(&xml::wrap(
        "ListAccessKeysResponse",
        &xml::wrap(
            "ListAccessKeysResult",
            &xml::wrap("AccessKeyMetadata", &members),
        ),
    )))
}

async fn delete_access_key(params: &str, state: SharedState) -> Response {
    let access_key_id = param(params, "AccessKeyId").unwrap_or_default();
    let mut st = state.write().await;
    st.access_keys.remove(&access_key_id);
    xml_response(xml::xml_doc(&xml::wrap("DeleteAccessKeyResponse", "")))
}

async fn get_access_key_last_used(params: &str, state: SharedState) -> Response {
    let access_key_id = param(params, "AccessKeyId").unwrap_or_default();
    let st = state.read().await;

    let last_used = st.access_keys.get(&access_key_id);
    let last_used_info = match last_used {
        Some(key) => {
            let date = key
                .last_used_date
                .as_deref()
                .unwrap_or("N/A");
            let service = key
                .last_used_service
                .as_deref()
                .unwrap_or("N/A");
            format!(
                "{}{}",
                xml::el("LastUsedDate", date),
                xml::el("ServiceName", service),
            )
        }
        None => format!(
            "{}{}",
            xml::el("LastUsedDate", "N/A"),
            xml::el("ServiceName", "N/A"),
        ),
    };

    xml_response(xml::xml_doc(&xml::wrap(
        "GetAccessKeyLastUsedResponse",
        &xml::wrap(
            "GetAccessKeyLastUsedResult",
            &xml::wrap("AccessKeyLastUsed", &last_used_info),
        ),
    )))
}
