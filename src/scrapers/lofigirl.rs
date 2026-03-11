use futures::{stream::FuturesOrdered, StreamExt};
use lazy_static::lazy_static;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::scrapers::get;

lazy_static! {
    static ref SELECTOR: Selector = Selector::parse("html > body > pre > a").unwrap();
}

async fn parse(client: &Client, path: &str) -> eyre::Result<Vec<String>> {
    let document = get(client, path, super::Source::Lofigirl).await?;
    let html = Html::parse_document(&document);

    Ok(html
        .select(&SELECTOR)
        .skip(5)
        .map(|x| String::from(x.attr("href").unwrap()))
        .collect())
}

async fn scan() -> eyre::Result<Vec<String>> {
    let client = Client::new();
    let items = parse(&client, "/").await?;

    let mut years: Vec<u32> = items
        .iter()
        .filter_map(|x| {
            let year = x.strip_suffix("/")?;
            year.parse().ok()
        })
        .collect();

    years.sort();

    let mut futures = FuturesOrdered::new();

    for year in years {
        let months = parse(&client, &year.to_string()).await?;

        for month in months {
            let client = client.clone();
            futures.push_back(async move {
                let path = format!("{}/{}", year, month);

                let items = parse(&client, &path).await.unwrap();
                items
                    .into_iter()
                    .filter_map(|x| {
                        if x.ends_with(".mp3") {
                            Some(format!("{path}{x}"))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<String>>()
            });
        }
    }

    let mut files = Vec::new();
    while let Some(mut result) = futures.next().await {
        files.append(&mut result);
    }

    eyre::Result::Ok(files)
}

pub async fn scrape() -> eyre::Result<()> {
    let files = scan().await?;
    for file in files {
        println!("{file}");
    }

    Ok(())
}
