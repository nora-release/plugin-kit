# Echo Example provider plugin

A provider plugin that answers every chat with your last message. It needs no
account and no network, so you can install it and see each part of a provider
plugin working in Nora.

```text
Examples/ProviderPlugin/
├── src/main.rs              the provider, built with nora-provider-plugin-sdk
└── package/                 everything Nora installs besides the executable
    ├── nora-provider.json   what the plugin is and what it adds
    └── ui/
        ├── panel.html       a panel in each provider's settings
        └── page.html        the plugin's own page
```

## Try it

```bash
make provider-example
```

Then in Nora, open **Plugins › Import Plugin › AI Providers** and choose
`.build/EchoExample`.

| Where | What you see | Declared by |
| --- | --- | --- |
| Model picker | **Echo**, answering "You said: …" | `models`, `fetch` in `main.rs` |
| Settings › AI Providers › Echo Example | **About this provider**, naming the provider it is open on and listing every provider | `ui.panels`, `ui.permissions` |
| Navigation rail | **Echo**, the plugin's page | `ui.page` |
| Any provider row's **…** menu | **Inspect with Echo**, opening the page with that provider | `ui.actions` |

## How the parts fit

- **The provider** (`main.rs`) is an executable Nora starts and talks to over
  JSONL; `serve` from the SDK handles the protocol. Nora sends each model
  request to `fetch` as the HTTP request it would have sent to `baseURL`. A
  real provider forwards it upstream with the account's credentials; Echo
  answers in the OpenAI chat completions format itself, streamed or not.
- **Panels and pages** are HTML files inside the package. Nora shows each in
  an isolated web view that loads only the package's files, reaches no
  network, and keeps no website data. The page gets `window.nora`:
  - `nora.onContext(fn)`: the provider it is open on (panels), the theme and
    the language;
  - `await nora.read(name)`: only the reads listed in `ui.permissions`;
    `providers:list` gives provider IDs and names, never credentials;
  - `nora.onAction(fn)`: the menu action that opened the page, with the
    provider it was picked on.

  A page that throws, or does not start within 10 seconds, is replaced by a
  card with Try Again.
- **Actions** are menu items that open the plugin's page, so a plugin with
  actions needs a page.
