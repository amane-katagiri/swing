use std::io::Write;

use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};

pub(super) struct Prepared {
    pub(super) etag: HeaderValue,
    gzip: Option<Gzipped>,
}

struct Gzipped {
    body: Bytes,
    etag: HeaderValue,
}

impl Prepared {
    pub(super) fn new(content_type: &str, body: &[u8]) -> Self {
        let compressible = content_type.starts_with("text/") || content_type == "image/svg+xml";
        let gzip = compressible
            .then(|| gzip(body))
            .filter(|gzipped| gzipped.len() < body.len())
            .map(|gzipped| Gzipped {
                etag: etag(&gzipped),
                body: Bytes::from(gzipped),
            });
        Self {
            etag: etag(body),
            gzip,
        }
    }

    pub(super) fn serve(
        &self,
        request: &HeaderMap,
        content_type: &str,
        body: &'static [u8],
    ) -> Response {
        let Some(gzipped) = &self.gzip else {
            return revalidated(request, content_type, self.etag.clone(), body);
        };
        let mut response = if accepts_gzip(request) {
            let mut response = revalidated(
                request,
                content_type,
                gzipped.etag.clone(),
                gzipped.body.clone(),
            );
            if response.status() == StatusCode::OK {
                response
                    .headers_mut()
                    .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            }
            response
        } else {
            revalidated(request, content_type, self.etag.clone(), body)
        };
        response
            .headers_mut()
            .insert(header::VARY, HeaderValue::from_static("accept-encoding"));
        response
    }
}

pub(in crate::dashboard) fn bytes_asset(
    request: &HeaderMap,
    content_type: &str,
    body: Bytes,
) -> Response {
    let etag = etag(&body);
    revalidated(request, content_type, etag, body)
}

fn gzip(body: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(body)
        .expect("writing to a Vec does not fail");
    encoder.finish().expect("writing to a Vec does not fail")
}

fn accepts_gzip(request: &HeaderMap) -> bool {
    request
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|coding| {
            let mut parts = coding.split(';');
            let name = parts.next().unwrap_or_default().trim();
            let quality = parts
                .filter_map(|p| p.trim().strip_prefix("q="))
                .find_map(|q| q.trim().parse::<f32>().ok())
                .unwrap_or(1.0);
            (name.eq_ignore_ascii_case("gzip") || name == "*") && quality > 0.0
        })
}

fn etag(body: &[u8]) -> HeaderValue {
    let digest = Sha256::digest(body);
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    HeaderValue::from_str(&format!("\"{hex}\"")).expect("hex etag is a valid header value")
}

fn is_fresh(request: &HeaderMap, etag: &HeaderValue) -> bool {
    let Some(tags) = request
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let etag = etag.to_str().unwrap_or_default();
    tags.split(',')
        .map(|tag| tag.trim())
        .any(|tag| tag == "*" || tag.strip_prefix("W/").unwrap_or(tag) == etag)
}

fn revalidated(
    request: &HeaderMap,
    content_type: &str,
    etag: HeaderValue,
    body: impl IntoResponse,
) -> Response {
    let mut response = if is_fresh(request, &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let content_type =
            HeaderValue::from_str(content_type).expect("content type is a valid header value");
        ([(header::CONTENT_TYPE, content_type)], body).into_response()
    };
    let headers = response.headers_mut();
    headers.insert(header::ETAG, etag);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

#[cfg(test)]
mod tests {
    use super::super::super::router;
    use super::super::super::test_support::*;
    use axum::body::Body;
    use axum::http::StatusCode;

    async fn get_with(path: &str, if_none_match: Option<&str>) -> axum::response::Response {
        let mut req = request("GET", path);
        if let Some(tag) = if_none_match {
            req = req.header("If-None-Match", tag);
        }
        call(router(test_state()), req.body(Body::empty()).unwrap()).await
    }

    #[tokio::test]
    async fn assets_are_revalidated_by_etag() {
        for path in [
            "/emoji.svg",
            "/",
            "/desktop-page.html",
            "/desktop-banner",
            "/mascots/index.json",
            "/mascots/yureko/sprite.png",
        ] {
            let resp = get_with(path, None).await;
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            assert_eq!(
                resp.headers().get("cache-control").unwrap(),
                "no-cache",
                "{path}"
            );
            let tag = resp
                .headers()
                .get("etag")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();

            let resp = get_with(path, Some(&tag)).await;
            assert_eq!(resp.status(), StatusCode::NOT_MODIFIED, "{path}");
            assert_eq!(resp.headers().get("etag").unwrap(), tag.as_str(), "{path}");
            assert!(resp.headers().get("content-type").is_none(), "{path}");
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            assert!(body.is_empty(), "{path}");

            let listed = format!("\"other\", W/{tag}");
            let resp = get_with(path, Some(&listed)).await;
            assert_eq!(resp.status(), StatusCode::NOT_MODIFIED, "{path}");

            let resp = get_with(path, Some("\"stale\"")).await;
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
        }
    }

    async fn get_encoded(
        path: &str,
        accept: &str,
        if_none_match: Option<&str>,
    ) -> axum::response::Response {
        let mut req = request("GET", path).header("Accept-Encoding", accept);
        if let Some(tag) = if_none_match {
            req = req.header("If-None-Match", tag);
        }
        call(router(test_state()), req.body(Body::empty()).unwrap()).await
    }

    #[tokio::test]
    async fn text_assets_are_gzipped_when_accepted() {
        use std::io::Read;
        for path in ["/emoji.svg", "/", "/app.js", "/style.css"] {
            let plain = get_with(path, None).await;
            assert!(plain.headers().get("content-encoding").is_none(), "{path}");
            assert_eq!(
                plain.headers().get("vary").unwrap(),
                "accept-encoding",
                "{path}"
            );
            let plain_tag = plain.headers().get("etag").unwrap().clone();
            let plain_body = axum::body::to_bytes(plain.into_body(), usize::MAX)
                .await
                .unwrap();

            let resp = get_encoded(path, "br, gzip;q=0.8", None).await;
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            assert_eq!(
                resp.headers().get("content-encoding").unwrap(),
                "gzip",
                "{path}"
            );
            assert_eq!(
                resp.headers().get("vary").unwrap(),
                "accept-encoding",
                "{path}"
            );
            let tag = resp.headers().get("etag").unwrap().clone();
            assert_ne!(tag, plain_tag, "{path}");
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            assert!(body.len() < plain_body.len(), "{path}");
            let mut decoded = Vec::new();
            flate2::read::GzDecoder::new(body.as_ref())
                .read_to_end(&mut decoded)
                .unwrap();
            assert_eq!(decoded, plain_body.as_ref(), "{path}");

            let resp = get_encoded(path, "gzip", Some(tag.to_str().unwrap())).await;
            assert_eq!(resp.status(), StatusCode::NOT_MODIFIED, "{path}");
            assert!(resp.headers().get("content-encoding").is_none(), "{path}");
            assert_eq!(
                resp.headers().get("vary").unwrap(),
                "accept-encoding",
                "{path}"
            );

            let resp = get_encoded(path, "gzip;q=0, deflate", None).await;
            assert!(resp.headers().get("content-encoding").is_none(), "{path}");
            assert_eq!(resp.headers().get("etag").unwrap(), plain_tag, "{path}");
        }
    }

    #[tokio::test]
    async fn binary_assets_are_not_gzipped() {
        for path in ["/favicon-32.png", "/fonts/pixelmplus12-regular.woff2"] {
            let resp = get_encoded(path, "gzip", None).await;
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            assert!(resp.headers().get("content-encoding").is_none(), "{path}");
            assert!(resp.headers().get("vary").is_none(), "{path}");
        }
    }
}
