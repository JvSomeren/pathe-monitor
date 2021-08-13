# Pathé monitor
A tool to watch [pathe.nl](https://pathe.nl) for ticket availability. Register the movies you want to monitor in the `config.json`-file.

It is recommended you use the Docker image in production.

Example `config.json`:
```json
{
  "requests": [
    {
      "cinema": "Spuimarkt",
      "date": "19-08-2021",
      "movie": "The Green Knight"
    }
  ]
}
```

## Development

## Generating release
```bash
$ DISCORD_WEBHOOK_URL="WEBHOOK_URL"
$ cargo run --package pathe-monitor --bin pathe-monitor
```

### Local
```bash
$ cargo build --release
```

### Docker
Building a new image
```bash
$ docker build -t registry.gitlab.com/jvsomeren/pathe-monitor .
```

Running the image
```bash
$ docker run --rm -e "DISCORD_WEBHOOK_URL"="WEBHOOK_URL" -v "${pwd}/config.json":"/app/config.json" registry.gitlab.com/jvsomeren/pathe-monitor
```

Publishing a new image
```bash
$ docker push registry.gitlab.com/jvsomeren/pathe-monitor
```
