extern crate chrono;
extern crate chrono_tz;

use log::{debug, error, info, trace, warn};

use ctrlc;
use reqwest::blocking::{Client, Response};
use serde_json::json;
use std::{
    env,
    fmt::Display,
    fs::File,
    io::{BufReader, BufWriter},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    vec,
};

use clokwerk::{Scheduler, TimeUnits};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};

// Defaults
const CONFIG_FILE: &str = "config.json";
const DEFAULT_LOG_LEVEL: &str = "Info";
const DEFAULT_TIMEZONE: &str = "Europe/Amsterdam"; // based on https://docs.rs/chrono-tz/0.5.3/chrono_tz/enum.Tz.html#variants

// START NOTIFICATIONS

#[derive(Serialize)]
struct DiscordNotificationField {
    name: String,
    value: String,
    inline: Option<bool>,
}

#[derive(Serialize)]
struct DiscordNotificationThumbnail {
    url: String,
}

#[derive(Serialize)]
struct DiscordNotificationFooter {
    text: String,
}

#[derive(Serialize)]
struct DiscordNotificationEmbed {
    title: String,
    description: Option<String>,
    url: String,
    fields: Vec<DiscordNotificationField>,
    thumbnail: DiscordNotificationThumbnail,
    footer: DiscordNotificationFooter,
}

#[derive(Serialize)]
struct DiscordNotification {
    content: String,
    embeds: Vec<DiscordNotificationEmbed>,
}

fn notify(client: &reqwest::blocking::Client, notification: DiscordNotification) {
    let webhook_url = env::var("DISCORD_WEBHOOK_URL").expect("missing `DISCORD_WEBHOOK_URL`-environment variable");
    info!(
        "Calling Discord webhook `{}` with payload:\n{}",
        webhook_url,
        json!(notification)
    );
    let res = client.post(webhook_url).json(&notification).send();

    if res.is_err() {
        error!("error calling webhook {:?}", res.err());
    }
}

// END NOTIFICATIONS

#[derive(Serialize, Deserialize, Clone, Debug)]
enum Cinema {
    Buitenhof = 7,
    Spuimarkt = 13,
    Delft = 18,
}

impl Display for Cinema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Cinema::Buitenhof => "Buitenhof",
            Cinema::Spuimarkt => "Spuimarkt",
            Cinema::Delft => "Delft",
        };
        f.write_str(&format!("Pathé {}", name))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct MovieMonitorRequest {
    cinema: Cinema,
    date: String,
    movie: String,
}

impl MovieMonitorRequest {
    fn api_url(&self) -> String {
        format!(
            "https://www.pathe.nl/cinema/schedules?cinemaId={cinema_id}&date={date}",
            cinema_id = self.cinema.clone() as i32,
            date = self.date
        )
    }
}

impl Display for MovieMonitorRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "'{movie}' op {date} in {cinema}",
            movie = self.movie,
            date = self.date,
            cinema = format!("{}", self.cinema),
        ))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct MovieMonitorConfig {
    requests: Vec<MovieMonitorRequest>,
}

fn generate_notification_field(time: ElementRef) -> DiscordNotificationField {
    let start_selector = Selector::parse("span.schedule-time__start").unwrap();
    let end_selector = Selector::parse("span.schedule-time__end").unwrap();
    let type_selector = Selector::parse("span.schedule-time__label").unwrap();

    let e_start = time.select(&start_selector).next().unwrap();
    let e_end = time.select(&end_selector).next().unwrap();
    let e_type = time.select(&type_selector).next().unwrap();

    let start = e_start.text().next().unwrap();
    let end = e_end.text().next().unwrap();
    let type_name = e_type.text().next().unwrap();

    let link = format!(
        "https://pathe.nl{}",
        time.value().attr("data-href").unwrap()
    );

    DiscordNotificationField {
        name: type_name.to_string(),
        value: format!("[{} - {}]({})", start, end, link),
        inline: Some(true),
    }
}

fn generate_notification(
    request: MovieMonitorRequest,
    item: ElementRef,
) -> Result<DiscordNotification, ()> {
    trace!("creating notification for {}", request);
    let MovieMonitorRequest {
        cinema,
        date,
        movie,
    } = request;

    let title_selector = Selector::parse("h4 a").unwrap();
    let thumbnail_selector = Selector::parse("div.schedule-simple__poster img").unwrap();
    let time_selector = Selector::parse("a.schedule-time").unwrap();

    let mut fields = vec![];

    let title_element = item.select(&title_selector).next().unwrap();

    for time in item.select(&time_selector) {
        let field = generate_notification_field(time);
        fields.push(field)
    }

    // fix potential misalignment
    if fields.len() > 3 && fields.len() % 3 == 2 {
        fields.push(DiscordNotificationField {
            name: ":rooster:".to_string(),
            value: ":popcorn:".to_string(),
            inline: Some(true),
        });
    }

    let e_thumbnail = item.select(&thumbnail_selector).next().unwrap();
    let thumbnail = e_thumbnail.value().attr("src").unwrap();

    let embed = DiscordNotificationEmbed {
        title: movie.to_string(),
        description: None,
        url: format!(
            "https://pathe.nl{}#agenda",
            title_element.value().attr("href").unwrap()
        ),
        fields,
        thumbnail: DiscordNotificationThumbnail {
            url: thumbnail.to_string(),
        },
        footer: DiscordNotificationFooter {
            text: "Generated by *pathe-monitor*".to_string(), // TODO dit dynamischer maken? om het terug te kunnen traceren
        },
    };

    Ok(DiscordNotification {
        content: format!(
            "Er zijn tickets beschikbaar voor '**{movie}**' op **{date}** in **{cinema}**.",
            movie = movie,
            date = date,
            cinema = format!("{}", cinema)
        ),
        embeds: vec![embed],
    })
}

fn check_response(
    request: MovieMonitorRequest,
    client: &Client,
    res: Response,
) -> Result<bool, ()> {
    debug!("handling {} response", request);

    let fragment = Html::parse_fragment(&res.text().unwrap());

    let scheduled_item_selector = Selector::parse("div.schedule-simple__item").unwrap();
    let title_selector = Selector::parse("h4 a").unwrap();

    for item in fragment.select(&scheduled_item_selector) {
        let title_element = item.select(&title_selector).next().unwrap();
        let title = title_element.text().next().unwrap();

        if title.to_lowercase() == request.movie.to_lowercase() {
            let notification = generate_notification(request, item).unwrap();
            notify(&client, notification);

            return Ok(true);
        }
    }

    Ok(false)
}

fn check_pending_movie_request(request: MovieMonitorRequest) -> Result<bool, ()> {
    info!("Processing {}", request);

    let client = reqwest::blocking::Client::new();
    let res = client.get(request.api_url()).send();

    if res.is_err() {
        error!("error calling {}. {:?}", request.api_url(), res.err());

        return Err(());
    }

    check_response(request, &client, res.unwrap())
}

fn read_config_from_file(path: &str) -> Result<MovieMonitorConfig, serde_json::Error> {
    trace!("reading config from `{}`", path);
    let file = File::open(path);

    if file.is_err() {
        warn!("`{}` not found, generating a fresh one", path);
        let config = MovieMonitorConfig { requests: vec![] };
        write_config_to_file(path, &config);

        return Ok(config);
    }

    let reader = BufReader::new(file.unwrap());
    serde_json::from_reader(reader)
}

fn write_config_to_file(path: &str, config: &MovieMonitorConfig) {
    debug!("writing new config to `{}`", path);
    let file = File::create(path);
    let writer = BufWriter::new(file.unwrap());

    serde_json::to_writer_pretty(writer, &config)
        .expect(format!("failed writing new `{}`", path).as_str());
}

fn check_pending_movie_requests() {
    let config = read_config_from_file(CONFIG_FILE).expect("failed reading `config.json`");

    info!("Processing {} movie requests", config.requests.len());
    for request in config.requests {
        match check_pending_movie_request(request.clone()) {
            Ok(true) => (),
            Ok(false) => info!("No tickets available for {}", request),
            Err(_) => error!("Something went wrong processing {}", request),
        };
    }

    // TODO write_config_to_file(CONFIG_FILE, &config);
}

fn setup_logger(log_level: log::LevelFilter) -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{datetime}][{target}][{level}] {message}",
                datetime = chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                target = record.target(),
                level = record.level(),
                message = message
            ))
        })
        .level(log_level)
        .level_for("reqwest::connect", log::LevelFilter::Off)
        .level_for("html5ever", log::LevelFilter::Off)
        .level_for("selectors", log::LevelFilter::Off)
        .level_for("rustls", log::LevelFilter::Off)
        .chain(std::io::stdout())
        // .chain(fern::log_file("output.log")?)
        .apply()?;
    trace!("initialized logger");

    Ok(())
}

fn setup_sig_handler(r: Arc<AtomicBool>) {
    ctrlc::set_handler(move || {
        trace!("ctrlc-handler called");
        r.store(false, Ordering::SeqCst);
    })
    .expect("failed setting Ctrl-C handler");
    trace!("initialized ctrlc-handler");
}

fn setup_scheduler() -> Result<Scheduler<chrono_tz::Tz>, String> {
    let tz: chrono_tz::Tz = env::var("TIMEZONE").unwrap_or(DEFAULT_TIMEZONE.to_string()).parse()?;
    let mut scheduler = Scheduler::with_tz(tz);
    debug!("initialized scheduler with TZ: '{:?}'", tz);
    info!(
        "Time in container is: {:?}",
        chrono::Local::now().with_timezone(&tz)
    );

    // TODO iedere dag een job met welke requests worden gemonitor

    // prepare config-file ahead of time
    read_config_from_file(CONFIG_FILE).ok();

    let job = scheduler
        .every(30.minutes())
        .run(check_pending_movie_requests);
    debug!("initialized job:\n{:?}", job);

    Ok(scheduler)
}

fn main() {
    let log_level =
        log::LevelFilter::from_str(&env::var("LOG_LEVEL").unwrap_or(DEFAULT_LOG_LEVEL.to_string()))
            .expect("invalid log level passed");
    setup_logger(log_level).expect("failed to initialize logging");

    info!("Pathé monitor is starting up!");

    let running = Arc::new(AtomicBool::new(true));

    setup_sig_handler(running.clone());
    // TODO validate env variables
    env::var_os("DISCORD_WEBHOOK_URL").expect("no `DISCORD_WEBHOOK_URL`-environment variable passed");

    let mut scheduler = setup_scheduler().expect("failed to initialize scheduler");

    while running.load(Ordering::SeqCst) {
        trace!("run pending jobs");
        scheduler.run_pending();
        trace!("finished pending jobs");

        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    info!("shutting down");
}
