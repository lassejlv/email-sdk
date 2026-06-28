use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::Url;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub(crate) struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct AwsSignRequest<'a> {
    pub credentials: &'a AwsCredentials,
    pub region: &'a str,
    pub service: &'a str,
    pub method: &'a str,
    pub url: &'a Url,
    pub body: &'a str,
    pub headers: BTreeMap<String, String>,
    pub now: DateTime<Utc>,
}

pub(crate) fn sign_aws_request(input: AwsSignRequest<'_>) -> BTreeMap<String, String> {
    let amz_date = input.now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = amz_date[..8].to_owned();
    let payload_hash = sha256_hex(input.body);
    let mut request_headers = input.headers;
    request_headers.insert("x-amz-content-sha256".to_owned(), payload_hash.clone());
    request_headers.insert("x-amz-date".to_owned(), amz_date.clone());
    if let Some(session_token) = &input.credentials.session_token {
        request_headers.insert("x-amz-security-token".to_owned(), session_token.clone());
    }

    let mut canonical_headers = BTreeMap::new();
    for (name, value) in &request_headers {
        canonical_headers.insert(name.to_lowercase(), value.trim().to_owned());
    }
    canonical_headers.insert(
        "host".to_owned(),
        input.url.host_str().unwrap_or_default().to_owned(),
    );

    let signed_headers = canonical_headers
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(";");
    let canonical_headers_text = canonical_headers
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect::<String>();
    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        date_stamp, input.region, input.service
    );
    let canonical_request = [
        input.method,
        input.url.path(),
        &canonical_query_string(input.url),
        &canonical_headers_text,
        &signed_headers,
        &payload_hash,
    ]
    .join("\n");
    let string_to_sign = [
        "AWS4-HMAC-SHA256",
        &amz_date,
        &credential_scope,
        &sha256_hex(&canonical_request),
    ]
    .join("\n");
    let signing_key = signing_key(
        &input.credentials.secret_access_key,
        &date_stamp,
        input.region,
        input.service,
    );
    let signature = hmac_hex(&signing_key, &string_to_sign);

    request_headers.insert(
        "Authorization".to_owned(),
        [
            format!(
                "AWS4-HMAC-SHA256 Credential={}/{}",
                input.credentials.access_key_id, credential_scope
            ),
            format!("SignedHeaders={signed_headers}"),
            format!("Signature={signature}"),
        ]
        .join(", "),
    );

    request_headers
}

fn canonical_query_string(url: &Url) -> String {
    let mut pairs = url
        .query_pairs()
        .map(|(key, value)| (aws_encode(&key), aws_encode(&value)))
        .collect::<Vec<_>>();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn aws_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn sha256_hex(value: &str) -> String {
    bytes_to_hex(&Sha256::digest(value.as_bytes()))
}

fn hmac_bytes(key: &[u8], value: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any size");
    mac.update(value.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn hmac_hex(key: &[u8], value: &str) -> String {
    bytes_to_hex(&hmac_bytes(key, value))
}

fn signing_key(secret_access_key: &str, date_stamp: &str, region: &str, service: &str) -> Vec<u8> {
    let date_key = hmac_bytes(format!("AWS4{secret_access_key}").as_bytes(), date_stamp);
    let region_key = hmac_bytes(&date_key, region);
    let service_key = hmac_bytes(&region_key, service);
    hmac_bytes(&service_key, "aws4_request")
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn signs_request_with_expected_headers() {
        let url =
            Url::parse("https://email.us-east-1.amazonaws.com/v2/email/outbound-emails").unwrap();
        let credentials = AwsCredentials {
            access_key_id: "AKIDEXAMPLE".to_owned(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_owned(),
            session_token: Some("token".to_owned()),
        };
        let headers = BTreeMap::from([("content-type".to_owned(), "application/json".to_owned())]);
        let signed = sign_aws_request(AwsSignRequest {
            credentials: &credentials,
            region: "us-east-1",
            service: "ses",
            method: "POST",
            url: &url,
            body: "{}",
            headers,
            now: Utc.with_ymd_and_hms(2020, 1, 2, 3, 4, 5).unwrap(),
        });

        assert_eq!(
            signed.get("x-amz-date").map(String::as_str),
            Some("20200102T030405Z")
        );
        assert!(signed.contains_key("x-amz-content-sha256"));
        assert_eq!(
            signed.get("x-amz-security-token").map(String::as_str),
            Some("token")
        );
        let authorization = signed.get("Authorization").unwrap();
        assert!(
            authorization.contains("Credential=AKIDEXAMPLE/20200102/us-east-1/ses/aws4_request")
        );
        assert!(authorization.contains(
            "SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token"
        ));
        assert!(authorization.contains("Signature="));
    }
}
