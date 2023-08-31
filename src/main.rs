
use axum::{
    body::Bytes,
    extract::{Multipart, DefaultBodyLimit, TypedHeader, Form},
    headers::{authorization, Origin},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post, get_service},
    BoxError, Router,
};
use askama::Template;
use futures::{Stream, TryStreamExt};
use std::{io, net::SocketAddr};
use tokio::{
    fs::File,
    io::{BufWriter, ErrorKind}
};
use tokio_util::io::StreamReader;
use tower_http::services::ServeDir;
use std::string::String;
use serde::Deserialize;

const UPLOADS_DIRECTORY: &str = "uploads";
const PASSWORDS: [&str;2] = ["dupa", "odbyt"];

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .init();

    match tokio::fs::create_dir(UPLOADS_DIRECTORY).await {
        Ok(_) => (),
        Err(err) => match err.kind() {
            ErrorKind::AlreadyExists => (),
            _ => panic!("{err:?}"),
        },
    }

    let app = Router::new()
        .route("/", get(index).post(index_post))
        .route("/list", post(list))
        .route_service("/:filename", get_service(ServeDir::new(UPLOADS_DIRECTORY)))
        .layer(DefaultBodyLimit::disable());

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn index() -> Result<Html<String>, String> {
    match tokio::fs::read_to_string("static/index.html").await {
        Ok(text) => Ok(Html(text)),
        Err(err) => Err(err.to_string()),
    }
}

async fn list(
    Form(pass): Form<Pass>,
) -> impl IntoResponse {

    if !PASSWORDS.contains(&pass.p.as_str()) {
        return (StatusCode::UNAUTHORIZED, "no password".to_string()).into_response()
    }

    let files = match std::fs::read_dir(UPLOADS_DIRECTORY) {
        Ok(list) => list,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }.filter_map(|f| {
        if let Ok(filenally) = f {
            if let Ok(filename) = filenally.file_name().into_string() {
                return Some(filename);
            }
        }
        None
    }).collect();

    let template = ListTemplate{files};
    HtmlTemplate(template).into_response()
}

#[derive(Deserialize, Debug)]
struct Pass {
    p: String,
}

#[derive(Template)]
#[template(path = "list.html")]
struct ListTemplate {
    files: Vec<String>,
}

struct HtmlTemplate<T>(T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template. Error: {}", err),
            )
                .into_response(),
        }
    }
}

async fn index_post(
    auth: TypedHeader<authorization::Authorization<authorization::Basic>>,
    origin: TypedHeader<Origin>,
    mut multipart: Multipart,
) -> Result<(StatusCode, String), (StatusCode, String)>
{
    if !PASSWORDS.contains(&auth.0.password()) {
        // return (StatusCode::UNAUTHORIZED, "mail uso@denpa.pl to maybe get a password").into_response()
        return Ok((StatusCode::UNAUTHORIZED, "mail uso@denpa.pl to maybe get a password".to_string()))
    }

    let mut urls: Vec<String> = Vec::new();
    while let Some(field) = multipart.next_field().await.unwrap() {
        let file_name = if let Some(file_name) = field.file_name() {
            file_name.to_owned()
        } else {
            continue;
        };

        if let Some(url) = std::path::Path::new(origin.0.to_string().as_str()).join(&file_name).to_str() {
            urls.push(url.to_string());
            stream_to_file(&file_name, field).await?;
        }
    }

    Ok((StatusCode::OK, urls.join("\n")))
}

async fn stream_to_file<S, E>(path: &str, stream: S) -> Result<(), (StatusCode, String)>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Into<BoxError>,
{
    if !path_is_valid(path) {
        return Err((StatusCode::BAD_REQUEST, "Invalid path".to_owned()));
    }

    async {
        // Convert the stream into an `AsyncRead`.
        let body_with_io_error = stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
        let body_reader = StreamReader::new(body_with_io_error);
        futures::pin_mut!(body_reader);

        // Create the file. `File` implements `AsyncWrite`.
        let path = std::path::Path::new(UPLOADS_DIRECTORY).join(path);
        let mut file = BufWriter::new(File::create(path).await?);

        // Copy the body into the file.
        tokio::io::copy(&mut body_reader, &mut file).await?;

        Ok::<_, io::Error>(())
    }
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
}

fn path_is_valid(path: &str) -> bool {
    let path = std::path::Path::new(path);
    let mut components = path.components().peekable();

    if let Some(first) = components.peek() {
        if !matches!(first, std::path::Component::Normal(_)) {
            return false;
        }
    }

    components.count() == 1
}
