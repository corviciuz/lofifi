use futures::{stream::FuturesOrdered, StreamExt};
use lazy_static::lazy_static;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::scrapers::{get, Source};

lazy_static! {
    static ref SELECTOR: Selector = Selector::parse("html > body > pre > a").unwrap();
}

async fn parse(client: &Client, path: &str) -> eyre::Result<Vec<String>> {
    let document = get(client, path, super::Source::Lofigirl).await?;
    let html = Html::parse_document(&document);

    Ok(html
        .select(&SELECTOR)
        .skip(1)
        .map(|x| String::from(x.attr("href").unwrap()))
        .collect())
}

async fn scan() -> eyre::Result<Vec<String>> {
    let client = Client::new();

    let mut releases = parse(&client, "/").await?;
    releases.truncate(releases.len() - 4);

    let mut futures = FuturesOrdered::new();

    for release in releases {
        let client = client.clone();
        futures.push_back(async move {
            let items = parse(&client, &release).await.unwrap();
            items
                .into_iter()
                .filter_map(|x| {
                    if x.ends_with(".mp3") {
                        Some(format!("{release}{x}"))
                    } else {
                        None
                    }
                })
                .collect::<Vec<String>>()
        });
    }

    let mut files = Vec::new();
    while let Some(mut result) = futures.next().await {
        files.append(&mut result);
    }

    eyre::Result::Ok(files)
}

pub async fn scrape() -> eyre::Result<()> {
    println!("{}/", Source::Lofigirl.url());
    let files = scan().await?;
    for file in files {
        println!("{file}");
    }

    Ok(())
}
