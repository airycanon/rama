use super::{
    data::{
        get_http_info, get_request_info, DataSource, FetchMode, Initiator, RequestInfo,
        ResourceType,
    },
    State,
};
use rama::{
    http::{
        response::Json,
        service::web::extract::{FromRequestParts, Path},
        Body, Request, Response, StatusCode,
    },
    service::Context,
};
use serde::Deserialize;
use serde_json::json;

type Html = rama::http::response::Html<String>;

fn html<T: Into<String>>(inner: T) -> Html {
    inner.into().into()
}

//------------------------------------------
// endpoints: navigations
//------------------------------------------

pub async fn get_root() -> Response {
    Response::builder()
        .status(StatusCode::TEMPORARY_REDIRECT)
        .header("Location", "/consent")
        .header("Set-Cookie", "rama-fp=0.2.0; Max-Age=60")
        .header(
            "Accept-Ch",
            [
                "Width",
                "Downlink",
                "Sec-CH-UA",
                "Sec-CH-UA-Mobile",
                "Sec-CH-UA-Full-Version",
                "ETC",
                "Save-Data",
                "Sec-CH-UA-Platform",
                "Sec-CH-Prefers-Reduced-Motion",
                "Sec-CH-UA-Arch",
                "Sec-CH-UA-Bitness",
                "Sec-CH-UA-Model",
                "Sec-CH-UA-Platform-Version",
                "Sec-CH-UA-Prefers-Color-Scheme",
                "Device-Memory",
                "RTT",
                "Sec-GPC",
            ]
            .join(", "),
        )
        .body(Body::empty())
        .expect("build redirect response")
}

pub async fn get_consent() -> Html {
    render_page(
        "🕵️ Fingerprint Consent",
        String::new(),
        format!(
            r##"<div class="consent">
                <div class="controls">
                    <a class="button" href="/report">Get Fingerprint Report</a>
                </div>
                <div class="small">
                    <p>
                        By clicking on the button above, you agree that we will store fingerprint information about your network traffic. We are only interested in the HTTP and TLS traffic sent by you. This information will be stored in a database for later processing.
                    </p>
                    <p>
                        Please note that we do not store IP information and we do not use third-party tracking cookies. However, it is possible that the telecom or hosting services used by you or us may track some personalized information, over which we have no control or desire. You can use utilities like the Unix `dig` command to analyze the traffic and determine what might be tracked.
                    </p>
                </div>
            </div>"##,
        ),
    )
}

pub async fn get_report(ctx: Context<State>, req: Request) -> Html {
    let http_info = get_http_info(&req);

    let (parts, _) = req.into_parts();

    let request_info = get_request_info(
        FetchMode::Navigate,
        ResourceType::Document,
        Initiator::Navigator,
        &ctx,
        &parts,
    )
    .await;

    let head = r#"<script src="/assets/script.js"></script>"#.to_owned();

    render_report(
        "🕵️ Fingerprint Report",
        head,
        String::new(),
        vec![
            ctx.state().data_source.clone().into(),
            request_info.into(),
            Table {
                title: "🚗 Http Headers".to_owned(),
                rows: http_info.headers,
            },
        ],
    )
}

//------------------------------------------
// endpoints: XHR
//------------------------------------------

#[derive(Deserialize)]
pub struct APINumberParams {
    number: usize,
}

pub async fn get_api_fetch_number(ctx: Context<State>, req: Request) -> Json<serde_json::Value> {
    let http_info = get_http_info(&req);

    let (parts, _) = req.into_parts();

    let request_info = get_request_info(
        FetchMode::SameOrigin,
        ResourceType::Xhr,
        Initiator::Fetch,
        &ctx,
        &parts,
    )
    .await;

    Json(json!({
        "number": ctx.state().counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        "fp": {
            "headers": http_info.headers,
            "request_info": request_info,
        }
    }))
}

pub async fn post_api_fetch_number(ctx: Context<State>, req: Request) -> Json<serde_json::Value> {
    let http_info = get_http_info(&req);

    let (parts, _) = req.into_parts();

    let number = match Path::<APINumberParams>::from_request_parts(&ctx, &parts).await {
        Ok(params) => params.number,
        Err(e) => {
            tracing::error!("Failed to parse number: {:?}", e);
            0
        }
    };

    let request_info = get_request_info(
        FetchMode::SameOrigin,
        ResourceType::Xhr,
        Initiator::Fetch,
        &ctx,
        &parts,
    )
    .await;

    Json(json!({
        "number": number,
        "fp": {
            "headers": http_info.headers,
            "request_info": request_info,
        }
    }))
}

pub async fn get_api_xml_http_request_number(
    ctx: Context<State>,
    req: Request,
) -> Json<serde_json::Value> {
    let http_info = get_http_info(&req);

    let (parts, _) = req.into_parts();

    let request_info = get_request_info(
        FetchMode::SameOrigin,
        ResourceType::Xhr,
        Initiator::Fetch,
        &ctx,
        &parts,
    )
    .await;

    Json(json!({
        "number": ctx.state().counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        "fp": {
            "headers": http_info.headers,
            "request_info": request_info,
        }
    }))
}

pub async fn post_api_xml_http_request_number(
    ctx: Context<State>,
    req: Request,
) -> Json<serde_json::Value> {
    let http_info = get_http_info(&req);

    let (parts, _) = req.into_parts();

    let number = match Path::<APINumberParams>::from_request_parts(&ctx, &parts).await {
        Ok(params) => params.number,
        Err(e) => {
            tracing::error!("Failed to parse number: {:?}", e);
            0
        }
    };

    let request_info = get_request_info(
        FetchMode::SameOrigin,
        ResourceType::Xhr,
        Initiator::Fetch,
        &ctx,
        &parts,
    )
    .await;

    Json(json!({
        "number": number,
        "fp": {
            "headers": http_info.headers,
            "request_info": request_info,
        }
    }))
}

//------------------------------------------
// endpoints: form
//------------------------------------------

pub async fn form(ctx: Context<State>, req: Request) -> Html {
    // TODO: get TLS Info (for https access only)
    // TODO: support HTTP1, HTTP2 and AUTO (for now we are only doing auto)

    let http_info = get_http_info(&req);

    let (parts, _) = req.into_parts();

    let request_info = get_request_info(
        FetchMode::SameOrigin,
        ResourceType::Form,
        Initiator::Form,
        &ctx,
        &parts,
    )
    .await;

    let mut content = String::new();

    content.push_str(r##"<a href="/report" title="Back to Home">🏠 Back to Home...</a>"##);

    if parts.method == "POST" {
        content.push_str(
            r##"<div id="input"><form method="GET" action="/form">
    <input type="hidden" name="source" value="web">
    <label for="turtles">Do you like turtles?</label>
    <select id="turtles" name="turtles">
        <option value="yes">Yes</option>
        <option value="no">No</option>
        <option value="maybe">Maybe</option>
    </select>
    <button type="submit">Submit</button>
</form></div>"##,
        );
    }

    render_report(
        "🕵️ Fingerprint Report » Form",
        String::new(),
        content,
        vec![
            ctx.state().data_source.clone().into(),
            request_info.into(),
            Table {
                title: "🚗 Http Headers".to_owned(),
                rows: http_info.headers,
            },
        ],
    )
}

//------------------------------------------
// endpoints: assets
//------------------------------------------

const STYLE_CSS: &str = include_str!("../assets/style.css");

pub async fn get_assets_style() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/css")
        .body(STYLE_CSS.into())
        .expect("build css response")
}

const SCRIPT_JS: &str = include_str!("../assets/script.js");

pub async fn get_assets_script() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/javascript")
        .body(SCRIPT_JS.into())
        .expect("build js response")
}

//------------------------------------------
// render utilities
//------------------------------------------

fn render_report(title: &'static str, head: String, mut html: String, tables: Vec<Table>) -> Html {
    html.push_str(r##"<div class="report">"##);
    for table in tables {
        html.push_str(&format!("<h2>{}</h2>", table.title));
        html.push_str("<table>");
        for (key, value) in table.rows {
            html.push_str(&format!(
                r##"<tr><td class="key">{}</td><td>{}</td></tr>"##,
                key, value
            ));
        }
        html.push_str("</table>");
    }
    html.push_str("</div>");
    render_page(title, head, html)
}

fn render_page(title: &'static str, head: String, content: String) -> Html {
    html(format!(
        r#"
        <!DOCTYPE html>
        <html lang="en">
        <head>
            <meta charset="UTF-8">
            <meta name="viewport" content="width=device-width, initial-scale=1.0">
        
            <title>ラマ | FP</title>
        
            <link rel="icon"
                href="data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%2210 0 100 100%22><text y=%22.90em%22 font-size=%2290%22>🦙</text></svg>">
        
            <meta name="description" content="rama proxy fingerprinting service">
            <meta name="robots" content="none">
            <link rel="canonical" href="https://ramaproxy.org/">
        
            <meta property="og:title" content="ramaproxy.org" />
            <meta property="og:locale" content="en_US" />
            <meta property="og:type" content="website">
            <meta property="og:description" content="rama proxy fingerprinting service" />
            <meta property="og:url" content="https://ramaproxy.org/" />
            <meta property="og:site_name" content="ramaproxy.org" />
            <meta property="og:image" content="https://raw.githubusercontent.com/plabayo/rama/main/docs/img/banner.svg">

            <link rel="stylesheet" type="text/css" href="/assets/style.css">

            {}
        </head>
        <body>
            <main>
                <h1>
                    <a href="/consent" title="rama-fp home">ラマ</a>
                    &nbsp;
                    |
                    &nbsp;
                    {}
                </h1>
                <div id="content">{}</div>
                <div id="input"></div>
                <div id="banner">
                    <a href="https://ramaproxy.org" title="rama proxy website">
                        <img src="https://raw.githubusercontent.com/plabayo/rama/main/docs/img/rama_banner.jpeg" alt="rama banner" />
                    </a>
                </div>
            </main>
        </body>
        </html>
    "#,
        head, title, content
    ))
}

impl From<RequestInfo> for Table {
    fn from(info: RequestInfo) -> Self {
        Self {
            title: "ℹ️ Request Info".to_owned(),
            rows: vec![
                ("User Agent".to_owned(), info.user_agent.unwrap_or_default()),
                ("Version".to_owned(), info.version),
                ("Scheme".to_owned(), info.scheme),
                ("Host".to_owned(), info.host.unwrap_or_default()),
                ("Method".to_owned(), info.method),
                ("Fetch Mode".to_owned(), info.fetch_mode.to_string()),
                ("Resource Type".to_owned(), info.resource_type.to_string()),
                ("Initiator".to_owned(), info.initiator.to_string()),
                ("Path".to_owned(), info.path),
                ("Uri".to_owned(), info.uri),
            ],
        }
    }
}

impl From<DataSource> for Table {
    fn from(data_source: DataSource) -> Self {
        Self {
            title: "📦 Data Source".to_owned(),
            rows: vec![
                ("Name".to_owned(), data_source.name),
                ("Version".to_owned(), data_source.version),
            ],
        }
    }
}

#[derive(Debug, Clone)]
struct Table {
    title: String,
    rows: Vec<(String, String)>,
}
