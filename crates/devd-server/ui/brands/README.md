# Brand marks

These are the marks of the services and providers devd actually supports. Each
one is the owner's own artwork.

They are served from the binary at `/brands/<name>.<ext>`, so the interface
never depends on a third party being reachable, never shows a mark that has
been tampered with, and works offline.

## Rules

* **A mark is never recoloured.** The colour is part of the brand.
* **A mark is never distorted.** Each one keeps its own proportions; a
  wordmark is given a wider slot rather than being squashed into a square.
* **A mark is never invented.** If devd cannot find an official mark for
  something it supports, that thing is shown without one rather than with
  something made up.
* **A mark is only shown for something devd supports.** A provider devd cannot
  reach has no mark, because a mark would be a claim.

## Where each one came from

| Mark | Source |
|---|---|
| `anthropic`, `claude`, `composer`, `deepseek`, `gemini`, `git`, `laravel`, `lmstudio`, `mistral`, `moonshot`, `mysql`, `nginx`, `node`, `ollama`, `openrouter`, `php`, `postgresql`, `redis`, `sqlite`, `sentry`, `docker`, `minio`, `meilisearch`, `elasticsearch`, `prometheus`, `grafana`, `cakephp`, `phpmyadmin`, `phpstorm`, `cursor`, `insomnia`, `postman`, `figma`, `discord`, `notion`, `obsidian`, `raycast`, `opencode` | The [simple-icons](https://simpleicons.org) registry, which curates brand marks and checks each one against the owner's guidelines before accepting it. Each file keeps the brand's own colour. |
| `openai` | OpenAI's own repository: `openai/openai-cookbook`, `openai-logomark.svg` |
| `zhipu` | https://zhipuai.cn/logo.svg |
| `xai` | The mark on https://x.ai/, as published on their own site |
| `groq` | https://groq.com/favicon.svg |
| `together` | https://www.together.ai/, from their own content delivery network |
| `fireworks` | https://fireworks.ai/icon0.svg |
| `typesense` | https://typesense.org/typesense-logo.svg, reduced to 128×128 |
| `valkey` | https://valkey.io/img/valkey-logo-og.png, reduced to 128×128 |
| `mailpit` | The Mailpit repository: `axllent/mailpit`, `server/ui/favicon.svg` |
| `xdebug` | https://xdebug.org/images/favicon.png |

## Formats

`.svg` wherever the owner publishes one; `.png` otherwise. The PNGs are at
128×128 or smaller so that the binary stays small and the mark is sharp at the
size it is shown.
