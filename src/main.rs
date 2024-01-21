use std::fs::File;
use std::io::Write;

use handlebars::Handlebars;
use komga::apis::configuration::{Configuration as KomgaConfiguration, Configuration};
use komga::models::{BookDto, SeriesDto};
use rocket::{get, Ignite, Rocket, routes, State};
use rocket::form::validate::Contains;
use rocket::fs::NamedFile;
use rocket::response::status::NotFound;
use serde::Deserialize;
use serenity::all::{ChannelId, ChannelType, GuildInfo};
use serenity::json::json;
use tokio_cron_scheduler::{Job, JobScheduler};

use settings::Settings;

mod settings;

const SERIES_STAT_PREFIX: &'static str = "📚Series: ";
const BOOKS_STAT_PREFIX: &'static str = "📖Issues: ";
#[derive(Debug, Deserialize)]
struct KomgaMetric {
    pub(crate) name: String,
    pub(crate) description: String,
    #[serde(rename = "baseUnit")]
    pub(crate) base_unit: String,
    pub(crate) measurements: Vec<KomgaMeasurement>,
    #[serde(rename = "availableTags")]
    pub(crate) available_tags: Vec<KomgaTag>,
}

#[derive(Debug, Deserialize)]
struct KomgaTag {
    pub(crate) tag: String,
    pub(crate) values: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct KomgaMeasurement {
    pub(crate) statistic: String,
    pub(crate) value: f32,
}

#[get("/recently_added/<date>")]
async fn komga_recently_added(content_dir: &State<String>, date: String) -> Result<NamedFile, NotFound<String>> {
    let path = format!("{}/komga/recently_added/{date}/index.html", content_dir);
    NamedFile::open(&path).await.map_err(|e| NotFound(e.to_string()))
}

#[get("/recently_added/<date>/thumbnails/<thumbnail>")]
async fn komga_recently_added_thumbnail(
    content_dir: &State<String>,
    date: String,
    thumbnail: String,
) -> Result<NamedFile, NotFound<String>> {
    let path = format!("{}/komga/recently_added/{date}/thumbnails/{thumbnail}", content_dir);
    NamedFile::open(&path).await.map_err(|e| NotFound(e.to_string()))
}

async fn rocket(settings: &Settings) -> Result<Rocket<Ignite>, anyhow::Error> {
    rocket::build()
        .manage(settings.newsletters.content_dir.clone())
        .mount("/", routes![komga_recently_added, komga_recently_added_thumbnail])
        .launch().await
        .map_err(|e| anyhow::anyhow!(e))
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let settings = Settings::new().expect("failed to load settings");
    let scheduler = JobScheduler::new().await.expect("failed to create scheduler");

    if settings.newsletters.enabled {
        // Write the Recently Added newsletter at startup, since we're not sure when the last time it was written
        write_komga_recently_added()
            .await
            .expect("failed to write Komga Recently Added");

        // Schedule the Recently Added newsletter
        if let Some(ref sched) = settings.newsletters.schedule {
            let job = Job::new_async(sched.as_str(), |_uuid, _l| Box::pin(async move {
                write_komga_recently_added()
                    .await
                    .expect("failed to write Komga Recently Added");
            })).expect("failed to add Komga Recent Updates job");
            scheduler.add(job)
                .await
                .expect("failed to schedule Komga Recent Updates job");
        }
    }


    if settings.stats.enabled {
        update_komga_stats().await
            .expect("failed to update Komga stats");
        // Schedule the Komga stats update
        let job = Job::new_async(settings.stats.schedule.as_str(), |_uuid, _l| Box::pin(async move {
            update_komga_stats().await
                .expect("failed to update Komga stats");
        })).expect("failed to add Komga Stats job");
        scheduler.add(job)
            .await
            .expect("failed to schedule Komga Stats job");
    }

    scheduler.start().await.expect("Job scheduler failed");
    rocket(&settings).await.expect("failed to start Rocket");
}

async fn update_komga_stats() -> Result<(), anyhow::Error> {
    let settings = Settings::new().expect("failed to load settings");
    let komga_http = reqwest::Client::from(&settings.komga);
    let series_stats = komga_http.get(
        format!("{}/actuator/metrics/komga.series", settings.komga.url).as_str())
        .send().await?
        .json::<KomgaMetric>().await?;
    let book_stats = komga_http.get(
        format!("{}/actuator/metrics/komga.books", settings.komga.url).as_str())
        .send().await?
        .json::<KomgaMetric>().await?;
    let discord_http = serenity::http::Http::new(&settings.discord.token);
    let category = find_or_create_channel(&discord_http, &settings.stats.category).await?;
    replace_stat_channels(&discord_http, &category, &series_stats.measurements.first(), &book_stats.measurements.first()).await
}

async fn replace_stat_channels(
    http: &serenity::http::Http,
    category: &ChannelId,
    series: &Option<&KomgaMeasurement>,
    books: &Option<&KomgaMeasurement>,
) -> Result<(), anyhow::Error> {
    let guild = get_guild(http).await?;
    let channels = http.get_channels(guild.id).await?;
    for channel in channels {
        if series.is_some() && channel.name.starts_with(SERIES_STAT_PREFIX) {
            channel.delete(http).await?;
        }
        if books.is_some() && channel.name.starts_with(BOOKS_STAT_PREFIX) {
            channel.delete(http).await?;
        }
    }
    if let Some(m) = series {
        let c = serenity::builder::CreateChannel::new(format!("{}{}", SERIES_STAT_PREFIX, m.value))
            .category(category)
            .kind(ChannelType::Voice);
        http.create_channel(guild.id, &c, None).await?;
    }
    if let Some(m) = books {
        let c = serenity::builder::CreateChannel::new(format!("{}{}", BOOKS_STAT_PREFIX, m.value))
            .category(category)
            .kind(ChannelType::Voice);
        http.create_channel(guild.id, &c, None).await?;
    }
    Ok(())
}

async fn write_komga_recently_added() -> Result<(), anyhow::Error> {
    // setup configuration variables
    let settings = Settings::new().expect("failed to load settings");
    let komga = KomgaConfiguration::from((&settings).komga.clone());
    let library_ids = get_library_ids(komga.clone(), &settings.komga.libraries)
        .await
        .expect("failed to get library ids");
    let date = chrono::Local::now().format("%Y-%m-%d")
        .to_string();
    let thumbnail_path = format!("{}/komga/recently_added/{}/thumbnails", settings.newsletters.content_dir, date);
    let week_ago = chrono::Local::now()
        .checked_sub_signed(chrono::Duration::days(8))
        .expect("failed to subtract 7 days");
    let formatted_date = week_ago.format("%Y-%m-%d").to_string();

    // Get the new series and books
    let new_series = get_recent_series(&komga, &library_ids, &week_ago).await?;
    let new_books = get_recent_books(&komga, library_ids, &week_ago, formatted_date).await?;

    // Create newsletter date and thumbnail directories
    std::fs::create_dir_all(thumbnail_path)?;
    // Write thumbnails locally, so that we don't have to jack with CORS on the Komga server
    write_series_thumbnails(&settings, &komga, &date, &new_series).await?;
    write_issue_thumbnails(&settings, komga, &date, &new_books).await?;

    // Write the newsletter index
    write_newsletter_index(&settings, &date, &new_series, &new_books)?;

    // Post the newsletter to Discord
    let http = serenity::http::Http::new(&settings.discord.token);
    let channel_id = get_discord_channel_id(&http, &settings.newsletters.channel)
        .await
        .expect("failed to get Discord channel id");
    let url = format!("{}/recently_added/{}", settings.newsletters.url, date);
    let image = format!("{}/recently_added/{}/thumbnails/series-{}.jpg", settings.newsletters.url, date, new_series.first().unwrap().id);
    let embed = serenity::builder::CreateEmbed::default()
        .title("Comics Weekly Update")
        .url(&url)
        .field("Date", date.clone(), true)
        .field("Series", new_series.len().to_string(), true)
        .field("Issues", new_books.len().to_string(), true)
        .timestamp(chrono::Utc::now())
        .thumbnail(&image)
        .image(&image)
        .description("Recently Added Comic series and issues");
    let create_message = serenity::builder::CreateMessage::default()
        .embed(embed);
    channel_id.send_message(&http, create_message).await?;
    Ok(())
}

fn write_newsletter_index(
    settings: &Settings,
    date: &str,
    new_series: &Vec<SeriesDto>,
    new_books: &Vec<BookDto>,
) -> Result<(), anyhow::Error> {
    let index_path = format!("{}/komga/recently_added/{date}/index.html", settings.newsletters.content_dir);
    let mut out = File::create(index_path)?;
    let mut handlebars = Handlebars::new();
    handlebars.register_template_file(
        "recently_added",
        format!("{}/komga/recently_added.html.hbs", settings.newsletters.templates_dir).as_str(),
    )
        .expect("failed to register recently_added template");
    let public_url = settings.komga.public_url.clone().unwrap_or(settings.komga.url.clone());
    let template_data = json!({
        "public_url": public_url,
        "series": new_series,
        "issues": new_books,
        "date": date,
    });
    let content = handlebars.render("recently_added", &template_data)?;
    out.write_all(content.as_bytes())?;
    out.flush()?;
    Ok(())
}

async fn get_recent_books(
    komga: &Configuration,
    library_ids: Option<Vec<String>>,
    week_ago: &chrono::DateTime<chrono::Local>,
    formatted_date: String,
) -> Result<Vec<BookDto>, anyhow::Error> {
    let mut new_books = komga::apis::book_controller_api::get_all_books(
        &komga, // configuration
        None, // search
        library_ids, // library_id
        None, // media_status
        None, // read_status
        Some(formatted_date), // released_after
        None, // tag
        Some(true), // unpaged
        None,  // page
        None, //size
        Some(vec!["created (desc)".to_string()]), // sort
    )
        .await?
        .content
        .expect("failed to get latest books");
    new_books.sort_by_cached_key(|issue| issue.created.clone());
    new_books.reverse();
    let new_books = new_books.iter()
        .take_while(|issue| {
            let created_at = chrono::DateTime::parse_from_rfc3339(issue.file_last_modified.as_str()).expect("failed to parse issue date");
            created_at.gt(&week_ago)
        })
        .map(|issue| issue.clone())
        .collect::<Vec<_>>();
    Ok(new_books)
}

async fn write_series_thumbnails(
    settings: &Settings,
    komga: &Configuration,
    date: &String,
    new_series: &Vec<SeriesDto>,
) -> Result<(), anyhow::Error> {
    for one_series in new_series {
        let thumbnail_filename = format!("{}/komga/recently_added/{}/thumbnails/series-{}.jpg", settings.newsletters.content_dir, date, one_series.id);
        let mut thumbnail_file = File::create(thumbnail_filename)?;
        let thumbnail_url = format!("{}/api/v1/series/{}/thumbnail", settings.komga.url, one_series.id);
        let thumbnail_content = komga.client
            .get(thumbnail_url.as_str())
            .header("Accept", "image/jpeg")
            .send().await?
            .bytes().await?;
        thumbnail_file.write_all(&thumbnail_content)?;
    }
    Ok(())
}

async fn write_issue_thumbnails(
    settings: &Settings,
    komga: Configuration,
    date: &String,
    new_books: &Vec<BookDto>,
) -> Result<(), anyhow::Error> {
    for book in new_books {
        let thumbnail_path = format!("{}/komga/recently_added/{}/thumbnails/book-{}.jpg", settings.newsletters.content_dir, &date, book.id);
        let mut thumbnail_file = File::create(thumbnail_path)?;
        let thumbnail_url = format!("{}/api/v1/books/{}/thumbnail", settings.komga.url, book.id);
        let thumbnail_content = komga.client
            .get(thumbnail_url.as_str())
            .header("Accept", "image/jpeg")
            .send().await?
            .bytes().await?;
        thumbnail_file.write_all(&thumbnail_content)?;
    }
    Ok(())
}

async fn get_recent_series(
    komga: &Configuration,
    library_ids: &Option<Vec<String>>,
    week_ago: &chrono::DateTime<chrono::Local>,
) -> Result<Vec<SeriesDto>, anyhow::Error> {
    let new_series = komga::apis::series_controller_api::get_new_series(
        &komga, // configuration
        library_ids.clone(), // library_id
        None, // deleted
        None, // oneshot
        Some(true), // unpaged
        None, // page
        None, // size
    )
        .await?
        .content
        .expect("failed to get latest series")
        .into_iter()
        .take_while(|series| {
            let created_at = chrono::DateTime::parse_from_rfc3339(series.created.as_str()).expect("failed to parse series date");
            created_at.gt(&week_ago)
        })
        .collect::<Vec<_>>();
    Ok(new_series)
}

async fn get_library_ids(
    komga_config: KomgaConfiguration,
    library_names: &Option<Vec<String>>,
) -> Result<Option<Vec<String>>, anyhow::Error> {
    if library_names.is_none() {
        return Ok(None);
    }
    let libraries = komga::apis::library_controller_api::get_all2(&komga_config).await?;
    let library_ids = libraries.iter().filter_map(|library| {
        match &library_names.contains(&library.name) {
            true => Some(library.id.clone()),
            false => None
        }
    }).collect::<Vec<_>>();
    Ok(Some(library_ids))
}

async fn get_discord_channel_id(
    http: &serenity::http::Http,
    discord_channel: &str,
) -> Result<ChannelId, anyhow::Error> {
    let guild = get_guild(http).await?;
    let channel_id = http
        .get_channels(guild.id).await?
        .iter().find(|channel| channel.name == discord_channel)
        .ok_or(anyhow::anyhow!("failed to find Komga channel"))?
        .id;
    Ok(channel_id)
}

async fn find_or_create_channel(
    http: &serenity::http::Http,
    category_name: &str,
) -> Result<ChannelId, anyhow::Error> {
    let guild = get_guild(http).await?;
    let maybe_channel = http.get_channels(guild.id).await?
        .into_iter().find(|channel| channel.name == category_name);
    match maybe_channel {
        Some(channel) => Ok(channel.id),
        None => {
            let create_category = serenity::builder::CreateChannel::new(category_name)
                .kind(ChannelType::Category);
            Ok(http.create_channel(guild.id, &create_category, None).await?.id)
        }
    }
}

async fn get_guild(http: &serenity::http::Http) -> Result<GuildInfo, anyhow::Error> {
    let guilds = http.get_guilds(None, None).await?;
    let guild = guilds
        .first()
        .ok_or(anyhow::anyhow!("failed to get guild"))?;
    Ok(guild.clone())
}

#[cfg(test)]
mod test {}