//! Live smoke test against the real site (1 request/s).
//!
//! cargo run -p parfumo-client --example live -- <command> <arg> [page|offset] [extra]
//!   search <text> [page] [brand=..,perfumer=..,notes=a|b,year=..,from=..,to=..,gender=men,sort=year_desc]
//!   perfume|similar <Brand/Perfume or URL>
//!   reviews|statements <Brand/Perfume> [offset] [limit]
//!   brand|perfumer|note <name, slug or URL> [page]
//!   rankings <top_men|top_women|top_unisex|new|trends|trends_wished|trends_worn|YEAR> [page]
//!   fetch <path or URL>
//!   alive | logged_in

use parfumo_client::*;

fn print<T: serde::Serialize>(value: Result<T, ClientError>) {
    match value {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).expect("serializable")),
        Err(e) => {
            eprintln!("ERROR: {e}");
            std::process::exit(1);
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let arg = args.get(1).map(String::as_str).unwrap_or("");
    let num = |i: usize, default: u32| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(default);
    let mut cfg = ClientConfig::default();
    if let Ok(base) = std::env::var("PARFUMO_BASE_URL") {
        cfg.base_url = base;
    }
    let client = ParfumoClient::new(cfg, GlobalLimiter::new(2), None).expect("client");
    match cmd {
        "search" => {
            let mut q = SearchQuery { text: Some(arg.to_string()).filter(|t| !t.is_empty()), ..Default::default() };
            for kv in args.get(3).map(String::as_str).unwrap_or("").split(',').filter(|s| !s.is_empty()) {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                match k {
                    "brand" => q.brand = Some(v.into()),
                    "perfumer" => q.perfumer = Some(v.into()),
                    "notes" => q.notes = v.split('|').map(String::from).collect(),
                    "year" => q.year = v.parse().ok(),
                    "from" => q.year_from = v.parse().ok(),
                    "to" => q.year_to = v.parse().ok(),
                    "gender" => q.genders = serde_json::from_str(&format!("[\"{v}\"]")).expect("gender"),
                    "sort" => q.sort = serde_json::from_str(&format!("\"{v}\"")).expect("sort"),
                    other => panic!("unknown filter {other}"),
                }
            }
            print(client.search_perfumes(&q, num(2, 1)).await)
        }
        "perfume" => print(client.get_perfume(arg).await),
        "reviews" => print(client.get_perfume_reviews(arg, ReviewSort::Newest, num(2, 0), num(3, 3)).await),
        "statements" => print(client.get_perfume_statements(arg, ReviewSort::Newest, num(2, 0), num(3, 5)).await),
        "similar" => print(client.get_similar_perfumes(arg).await),
        "brand" => print(client.get_brand(arg, num(2, 1)).await),
        "perfumer" => print(client.get_perfumer(arg, num(2, 1)).await),
        "note" => print(client.get_note(arg, num(2, 1)).await),
        "rankings" => {
            let kind = match arg {
                "top_men" => RankingKind::TopMen,
                "top_women" => RankingKind::TopWomen,
                "top_unisex" => RankingKind::TopUnisex,
                "new" => RankingKind::New,
                "trends" => RankingKind::Trends,
                "trends_wished" => RankingKind::TrendsWished,
                "trends_worn" => RankingKind::TrendsWorn,
                year => RankingKind::Year(year.parse().expect("ranking kind or year")),
            };
            print(client.get_rankings(&kind, num(2, 1)).await)
        }
        "fetch" => print(client.fetch_page(arg).await),
        "alive" => print(client.csrf_key().await.map(|k| format!("csrf_key received ({} chars)", k.len()))),
        "logged_in" => print(client.is_logged_in().await),
        _ => {
            eprintln!("usage: live <search|perfume|reviews|statements|similar|brand|perfumer|note|rankings|fetch|alive|logged_in> <arg>");
            std::process::exit(2);
        }
    }
}
