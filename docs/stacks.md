# Supported stacks and compatibility matrix

<!-- Generated from crates/devd-stacks/defs by `devd stacks matrix --markdown`. A test fails if this file is stale. Do not edit by hand. -->

Every stack's commands and flags come from its official documentation or CLI help. **Verified** says which: `cli-help` means the flags were read from the tool's own `--help` output at the version shown; `docs` means they follow the official documentation (the tool was not available to run).

## Stacks

| Stack | Language | Versions (tool requirements) | Package managers | Verified | Official docs |
|---|---|---|---|---|---|
| Next.js | javascript | Next.js 16 (latest) (default) — node ≥ 20.9 | npm, pnpm, yarn, bun | cli-help: create-next-app 16.4.2 (2026-10-20) | [Installation](https://nextjs.org/docs/app/getting-started/installation) |
| Nuxt | javascript | Nuxt 4 (latest) (default) — node ≥ 22.22.3 | npm, pnpm, yarn, bun | cli-help: nuxi 4.0.1 / nuxt 4.6.2 (2026-10-20) | [Installation](https://nuxt.com/docs/getting-started/installation) |
| Astro | javascript | Astro (latest) (default) — node ≥ 22.22 | npm, pnpm, yarn, bun | cli-help: create-astro 5.3.0 (2026-10-20) | [Install Astro](https://docs.astro.build/en/install-and-setup/) |
| React (Vite) | javascript | Vite (latest) (default) — node ≥ 20.19 | npm, pnpm, yarn, bun | cli-help: create-vite 9.3.0 (2026-10-20) | [Vite - Scaffolding your first project](https://vite.dev/guide/#scaffolding-your-first-vite-project) |
| Vue | javascript | Vue 3 (latest) (default) — node ≥ 22.18 | npm, pnpm, yarn, bun | cli-help: create-vue 3.24.0 (2026-10-20) | [Quick Start](https://vuejs.org/guide/quick-start.html) |
| SvelteKit | javascript | SvelteKit (latest) (default) — node ≥ 20 | npm, pnpm, yarn, bun | cli-help: sv 1.1.1 (2026-10-20) | [Creating a project](https://svelte.dev/docs/kit/creating-a-project) |
| Express | javascript | express-generator (Express 4) (default) — node ≥ 18 | npm, pnpm, yarn, bun | cli-help: express-generator 4.16.1 (2026-10-20) | [Installing Express](https://expressjs.com/en/starter/installing.html) |
| Node.js | javascript | Node.js LTS (default) — node ≥ 20 | npm, pnpm, yarn, bun | docs: npm 10 / Node.js 22 (2026-10-20) | [Node.js documentation](https://nodejs.org/docs/latest/api/) |
| Laravel | php | Laravel 13 (latest) (default) — php ≥ 8.3, composer ≥ 2.2<br>Laravel 12 — php ≥ 8.2, composer ≥ 2.2<br>Laravel 11 — php ≥ 8.2, composer ≥ 2.2 | composer | cli-help: composer 2.x create-project; laravel/laravel v13.11 / v12.12 / v11.6 on Packagist (2026-10-20) | [Installation](https://laravel.com/docs/installation) |
| Slim | php | Slim 4 (default) — php ≥ 7.4, composer ≥ 2.2 | composer | docs: slim/slim-skeleton 4.5 on Packagist (2026-10-20) | [Slim installation](https://www.slimframework.com/docs/v4/start/installation.html) |
| Django | python | Django 6.1 (latest) (default) — python ≥ 3.12<br>Django 5.2 (LTS) — python ≥ 3.10<br>Django 4.2 (LTS — python ≥ 3.8 | pip, uv | docs: Django 5.2.18 / 6.0.9 / 6.1.2 on PyPI (Requires-Python read from PyPI) (2026-10-20) | [Tutorial - writing your first app](https://docs.djangoproject.com/en/stable/intro/tutorial01/) |
| Flask | python | Flask 3.1 (latest) (default) — python ≥ 3.9 | pip, uv | docs: Flask 3.1.3 on PyPI (Requires-Python >=3.9) (2026-10-20) | [Installation](https://flask.palletsprojects.com/en/stable/installation/) |
| FastAPI | python | FastAPI (latest) (default) — python ≥ 3.10 | pip, uv | docs: fastapi 0.150.0 on PyPI (Requires-Python >=3.10); the `fastapi` CLI ships with fastapi[standard] (2026-10-20) | [Virtual environments](https://fastapi.tiangolo.com/virtual-environments/) |
| Python | python | Python 3 (default) — python ≥ 3.9 | uv, pip | cli-help: uv 0.11 init --help; python -m venv --help (3.13) (2026-10-20) | [Python tutorial - virtual environments](https://docs.python.org/3/tutorial/venv.html) |
| Ruby on Rails | ruby | Rails 8.1 (latest) (default) — ruby ≥ 3.2, rails 8.1.x<br>Rails 8.0 — ruby ≥ 3.2, rails 8.0.x<br>Rails 7.2 — ruby ≥ 3.1, rails 7.2.x | bundler | cli-help: rails 8.1.4 `rails new --help` (2026-10-20) | [Getting started](https://guides.rubyonrails.org/getting_started.html) |
| Go | go | Installed Go toolchain (default) — go ≥ 1.22 | go | docs: go 1.22+ `go mod init`, `go test`, `go vet` (2026-10-20) | [Tutorial - Get started with Go](https://go.dev/doc/tutorial/getting-started) |
| Rust | rust | Installed Rust toolchain (default) — rust ≥ 1.85 | cargo | cli-help: cargo 1.97.0 `cargo new --help` (2026-10-20) | [The Cargo Book - Creating a new package](https://doc.rust-lang.org/cargo/guide/creating-a-new-project.html) |
| Spring Boot | java | Current Spring Boot (start.spring.io default) (default) — java ≥ 17 | — | docs: Spring Initializr API guide (2026-10-20) | [Spring Initializr API](https://docs.spring.io/initializr/docs/current/reference/html/#api-guide) |
| ASP.NET Core | csharp | .NET 10 (LTS) (default) — dotnet ≥ 10.0<br>.NET 9 — dotnet ≥ 9.0<br>.NET 8 (LTS) — dotnet ≥ 8.0 | dotnet | docs: dotnet new templates (webapi, mvc, webapp, blazor, console, classlib) (2026-10-20) | [Tutorial - Create a web API](https://learn.microsoft.com/aspnet/core/tutorials/first-web-api) |
| Expo (React Native) | javascript | Expo SDK (latest) (default) — node ≥ 22.13 | npm, pnpm, yarn, bun | cli-help: create-expo-app 5.0.0 / expo 58 (2026-10-20) | [Create a project](https://docs.expo.dev/get-started/create-a-project/) |
| React Native (CLI) | javascript | React Native (latest) (default) — node ≥ 22.13 | npm, yarn, bun | cli-help: @react-native-community/cli 20.2.0 `init --help` (2026-10-20) | [Set up your environment](https://reactnative.dev/docs/set-up-your-development-environment) |
| Ruby *(detect and run only)* | ruby | Installed Ruby (default) — ruby ≥ 3.1 | bundler | docs: bundler 4.0 (2026-10-20) | [Ruby documentation](https://www.ruby-lang.org/en/documentation/) |
| Java (Maven / Gradle) *(detect and run only)* | java | Installed JDK (default) — java ≥ 17 | maven, gradle | docs: Maven 3.9 / Gradle 8 (2026-10-20) | [Maven in 5 minutes](https://maven.apache.org/guides/getting-started/maven-in-five-minutes.html) |

## Tools and services

| Tool | Category | Supported from | Install with | Service port |
|---|---|---|---|---|
| Git | vcs | 2.30 | apt, dnf, pacman, brew, winget | — |
| Node.js | runtime | 20.9 | devd-runtime, brew, winget, apt, dnf, pacman | — |
| npm | package-manager | — | manual | — |
| pnpm | package-manager | — | npm, brew, winget | — |
| Yarn | package-manager | — | npm, brew, winget | — |
| Bun | package-manager | — | brew, winget, npm | — |
| PHP | runtime | 8.1 | devd-runtime, apt, dnf, pacman, brew | — |
| Composer | package-manager | 2.2 | apt, dnf, pacman, brew, winget | — |
| Python | runtime | 3.9 | apt, dnf, pacman, brew, winget | — |
| uv | package-manager | — | brew, winget, manual | — |
| Poetry | package-manager | — | brew, manual | — |
| Ruby | runtime | 3.1 | apt, dnf, pacman, brew, winget | — |
| Bundler | package-manager | — | gem | — |
| Rails | package-manager | — | gem | — |
| Go | runtime | 1.22 | devd-runtime, brew, winget, apt, dnf, pacman | — |
| JDK | runtime | 17 | apt, dnf, pacman, brew, winget | — |
| Maven | build | — | apt, dnf, pacman, brew, manual | — |
| Gradle | build | — | apt, dnf, pacman, brew, winget | — |
| .NET SDK | runtime | 8.0 | winget, brew, apt, dnf | — |
| Rust (cargo) | runtime | 1.75 | brew, winget, apt, dnf, pacman | — |
| MariaDB | database | — | apt, dnf, pacman, brew, winget | 3306 |
| MySQL | database | — | apt, dnf, brew, winget | 3306 |
| PostgreSQL | database | — | apt, dnf, pacman, brew, winget | 5432 |
| Redis | database | — | apt, dnf, pacman, brew, manual | 6379 |
| SQLite | database | — | apt, dnf, pacman, brew, manual | — |
| Docker | container | — | apt, dnf, pacman, brew-cask, winget | 2375 |
| nginx | utility | — | apt, dnf, pacman, brew | — |
| GitHub CLI | vcs | — | apt, dnf, pacman, brew, winget | — |
| Android Debug Bridge | mobile | — | apt, dnf, pacman, brew-cask, winget | — |
| Android SDK command-line tools | mobile | — | brew-cask, winget, manual | — |
| Expo CLI | mobile | — | per project | — |
| EAS CLI | mobile | — | npm | — |
| C/C++ build tools | build | — | apt, dnf, pacman, manual | — |
| Archive tools (zip/unzip) | utility | — | apt, dnf, pacman, brew | — |
| ImageMagick | utility | — | apt, dnf, pacman, brew, winget | — |
| libvips | utility | — | apt, dnf, pacman, brew | — |

## Operating systems

The engine runs on Windows 10/11 x64 and Linux x64. Install recipes exist per package manager (`apt`, `dnf`, `pacman`, `brew`, `winget`, `npm`, `gem`) and devd uses the ones it finds; where none applies the plan says so and gives the manual steps instead of pretending.
