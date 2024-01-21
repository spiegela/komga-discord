# Komga Discord Bot

A Discord bot for Komga.

## Features

The bot can do the following:

* Send a periodic message to the channel of your choice with newly added series and issues.
* Create statistic channels for series and issues counts

## Requirements

* [Komga](https://komga.org/) (obviously)
* A Discord server
* A Discord bot token (see [this guide](https://discordjs.guide/preparations/setting-up-a-bot-application.html#creating-your-bot))

## Installation

### Docker

The easiest way to run the bot is to use the [Docker image](https://hub.docker.com/r/spiegela/komga-discord-bot).

```shell
docker run -d \
  -v <path to config>:/config \
  -v <path to newsletters>:/newsletters \
  -p 8080:8080 \
  -e ROCKET_ADDRESS=0.0.0.0 \
  -e DISCORD.TOKEN=<token> \
  -e KOMGA.URL=<public url> \
  -e KOMGA.USERNAME=<admin username> \
  -e KOMGA.PASSWORD=<password> \
  spiegela/komga-discord-bot:latest
```

### Kubernetes / Helm

A Helm chart is available in the [helm](helm) directory. It can be installed with the following command:

```shell
helm install komga-discord-bot ./helm/komga-discord \
  --set discord.token=<token> \
  --set komga.url=<public url> --set komga.username=<admin username> --set komga.password=<password>
```

For more information, see the [`values.yaml`](helm/komga-discord/values.yaml).