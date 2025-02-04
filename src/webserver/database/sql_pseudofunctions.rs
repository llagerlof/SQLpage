use std::borrow::Cow;

use actix_web::http::StatusCode;
use actix_web_httpauth::headers::authorization::Basic;
use sqlparser::ast::FunctionArg;

use crate::webserver::{
    http::{RequestInfo, SingleOrVec},
    ErrorWithStatus,
};

use super::sql::{
    extract_integer, extract_single_quoted_string, extract_variable_argument, FormatArguments,
};
use anyhow::{anyhow, Context};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum StmtParam {
    Get(String),
    Post(String),
    GetOrPost(String),
    Cookie(String),
    Header(String),
    Error(String),
    BasicAuthPassword,
    BasicAuthUsername,
    HashPassword(Box<StmtParam>),
    RandomString(usize),
}

pub(super) fn func_call_to_param(func_name: &str, arguments: &mut [FunctionArg]) -> StmtParam {
    match func_name {
        "cookie" => extract_single_quoted_string("cookie", arguments)
            .map_or_else(StmtParam::Error, StmtParam::Cookie),
        "header" => extract_single_quoted_string("header", arguments)
            .map_or_else(StmtParam::Error, StmtParam::Header),
        "basic_auth_username" => StmtParam::BasicAuthUsername,
        "basic_auth_password" => StmtParam::BasicAuthPassword,
        "hash_password" => extract_variable_argument("hash_password", arguments)
            .map(Box::new)
            .map_or_else(StmtParam::Error, StmtParam::HashPassword),
        "random_string" => extract_integer("random_string", arguments)
            .map_or_else(StmtParam::Error, StmtParam::RandomString),
        unknown_name => StmtParam::Error(format!(
            "Unknown function {unknown_name}({})",
            FormatArguments(arguments)
        )),
    }
}

pub(super) fn extract_req_param<'a>(
    param: &StmtParam,
    request: &'a RequestInfo,
) -> anyhow::Result<Option<Cow<'a, str>>> {
    Ok(match param {
        StmtParam::Get(x) => request.get_variables.get(x).map(SingleOrVec::as_json_str),
        StmtParam::Post(x) => request.post_variables.get(x).map(SingleOrVec::as_json_str),
        StmtParam::GetOrPost(x) => request
            .post_variables
            .get(x)
            .or_else(|| request.get_variables.get(x))
            .map(SingleOrVec::as_json_str),
        StmtParam::Cookie(x) => request.cookies.get(x).map(SingleOrVec::as_json_str),
        StmtParam::Header(x) => request.headers.get(x).map(SingleOrVec::as_json_str),
        StmtParam::Error(x) => anyhow::bail!("{}", x),
        StmtParam::BasicAuthPassword => extract_basic_auth_password(request)
            .map(Cow::Borrowed)
            .map(Some)?,
        StmtParam::BasicAuthUsername => extract_basic_auth_username(request)
            .map(Cow::Borrowed)
            .map(Some)?,
        StmtParam::HashPassword(inner) => extract_req_param(inner, request)?
            .map_or(Ok(None), |x| hash_password(&x).map(Cow::Owned).map(Some))?,
        StmtParam::RandomString(len) => Some(Cow::Owned(random_string(*len))),
    })
}

fn random_string(len: usize) -> String {
    use rand::{distributions::Alphanumeric, Rng};
    password_hash::rand_core::OsRng
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn hash_password(password: &str) -> anyhow::Result<String> {
    let phf = argon2::Argon2::default();
    let salt = password_hash::SaltString::generate(&mut password_hash::rand_core::OsRng);
    let password_hash = &password_hash::PasswordHash::generate(phf, password, &salt)
        .map_err(|e| anyhow!("Unable to hash password: {}", e))?;
    Ok(password_hash.to_string())
}

fn extract_basic_auth_username(request: &RequestInfo) -> anyhow::Result<&str> {
    Ok(extract_basic_auth(request)?.user_id())
}

fn extract_basic_auth_password(request: &RequestInfo) -> anyhow::Result<&str> {
    let password = extract_basic_auth(request)?.password().ok_or_else(|| {
        anyhow::Error::new(ErrorWithStatus {
            status: StatusCode::UNAUTHORIZED,
        })
    })?;
    Ok(password)
}

fn extract_basic_auth(request: &RequestInfo) -> anyhow::Result<&Basic> {
    request
        .basic_auth
        .as_ref()
        .ok_or_else(|| {
            anyhow::Error::new(ErrorWithStatus {
                status: StatusCode::UNAUTHORIZED,
            })
        })
        .with_context(|| "Expected the user to be authenticated with HTTP basic auth")
}
