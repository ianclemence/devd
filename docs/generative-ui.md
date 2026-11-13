# Generative interface: drawing in answers

An assistant that can only write prose gives a table as pipes and a chart as an adjective. In devd it can
**draw**: it writes a small *spec*, a tree of components from a fixed *catalog*, and the window draws each one
with its own components, in whatever theme is on. A spec is data. It is checked before it is trusted, nothing in
it is run, and anything that cannot be drawn degrades to the code it is.

This is the same idea as Vercel's json-render and Chat SDK (a typed catalog, a JSON spec, streaming, validation
with issues sent back to the model), built for a native window and for devd's own authority model.

## The format

A spec travels inside a normal reply as a fenced block named `devd-ui` holding one JSON object. Every
component is `{"type": "Name", "props": {...}, "children": [...]}` and the root is always a `Card`.

````text
```devd-ui
{"type":"Card","props":{"title":"Sites","subtitle":"3 of 4 answering"},"children":[
  {"type":"Metrics","props":{"items":[{"label":"Answering","value":3,"delta":"-1"}]}},
  {"type":"Table","props":{"columns":["Site","State"],"rows":[["shop.test","up"],["blog.test","down"]]}},
  {"type":"Actions","children":[{"type":"Button","props":{"label":"Open sites","action":"open","target":"/sites"}}]}
]}
```
````

Because it is ordinary text in the reply it streams with the rest, works with every provider, is stored in the
conversation like any other message (so history and reload need nothing extra), and shows as a code block
anywhere nothing can draw it.

## The catalog

`crates/devd-genui` holds the catalog, and it is the single source of truth: the instructions the model is given
(`devd_genui::prompt()`), the checks (`check`, `draft`) and the window's renderer all read the same list.

| Component | For |
|---|---|
| `Card` | The root. A titled card; everything goes inside it. |
| `Section` | A titled group inside the card. |
| `Text`, `Callout` | A paragraph (markdown), and a notice that must not be missed. |
| `Fields`, `Table` | Labelled facts; rows and columns (a column of numbers right-aligns). |
| `Chart` | Bars: `bar`, `column` or `share` (parts of a whole). |
| `Metrics`, `Steps`, `Timeline`, `Details` | The visuals the window already drew from fenced `metrics`, `steps`, `timeline` and `details` blocks, now components too. |
| `Code`, `Diff`, `FileList` | Code or output; a patch drawn as a diff; files that open when clicked. |
| `Divider` | A thin line. |
| `Actions`, `Button`, `LinkButton` | Choices the person can make. |

Limits keep a spec from taking a minute to lay out: 300 components, 6 levels deep, 4 tables, 4 charts, 200 rows,
12 columns, 40 bars, 8 buttons, 6000 characters of text. Charts are bars only; line, area and pie charts are not
drawn.

## Streaming and checking

* **While it is written.** The window parses the growing block every time text arrives. `parse_partial` closes
  whatever is still open (a cut inside a key, a string or a number loses nothing before it) and `draft` keeps
  every component whose props already fit. The card appears as soon as the root exists, shows what has arrived,
  and a quiet bar shows more is coming. Every prefix of a valid spec parses without panicking and the picture
  only ever grows; that is tested.
* **When it is finished.** `check` returns one readable `Issue` per problem (`children[2].props.rows[0]: has 1
  cells but the table has 2 columns`). A finished spec with problems is drawn as far as it can be, with the line
  "Part of this could not be drawn".
* **Self-correction.** After a turn the agent checks every finished block. If one is wrong it sends the model a
  `[devd]` note listing each problem and the model writes the answer again, up to `agent.draw_fixes` times
  (default 2). In the transcript this is the line "devd asked it to draw that again". The same loop runs for the
  end-of-turn gate.

## Buttons

A button never does anything until the person presses it, and nothing it says is trusted to describe what it
does.

* **`reply`** sends the button's `text` as the person's next message, leaving what they were typing and what they
  had attached as it was.
* **`open`** goes to a devd page (`/sites`) or opens a file or folder that exists.
* **`run`** runs a capability by id. The id is looked up in what devd really offers; an unknown one does nothing
  and says so. A capability that only reads runs at once. Anything else opens a confirmation showing its id, what
  it does, its kind and its arguments, and then takes the same path as every other button, so tiers, approvals
  and the activity record all apply.
* **`LinkButton`** opens an `https` address in the browser.

## Turning it off

`agent.draw` (flag, on by default) removes the instructions and the checks. `agent.draw_fixes` sets how many
times a bad drawing is sent back (0 never). A block is still drawn if it arrives, whichever way it got there.

## Where it is

| | |
|---|---|
| `crates/devd-genui` | The catalog, spec, partial parser, checker, prompt, plain-text form. Pure and tested. |
| `crates/devd-app-core/src/markdown.rs` | `Block::Ui` and its state while streaming. |
| `crates/devd-app/src/ui/genui.rs` | The pictures, and the button actions. |
| `crates/devd-app/src/screens/genui_actions.rs` | What happens when a button is pressed. |
| `crates/devd-agent/src/chat.rs` | The instructions and the self-correction loop. |

# Site icons before links

A web link in an answer starts with the site's own icon, as in other assistants. The icon is fetched from the
site itself and not from a third party's icon service, which would be told every site the assistant mentions.

* `web.favicon` (a read capability, human callers) looks a host up through the one guarded fetch devd has: https
  only, public hosts only, every redirect re-checked. It reads the page's `<link rel="icon">` tags, falls back to
  `/favicon.ico`, and keeps the best file under the cache folder. A file is kept only if its first bytes say it is
  a picture the window can draw; an `.ico` is opened and its entry nearest 32 pixels taken out, bitmap entries
  re-encoded as PNG. A found icon is believed for 14 days and a miss for one day.
* In the window, room for the icon is reserved in the text (two figure spaces before the link) and the icon is
  painted into it once the text has been laid out, so it stays beside its link when the line wraps. Until the icon
  arrives, or when a site has none, a tile with the site's first letter stands in. A site on this machine or a
  private network (`.test`, `localhost`, an IP address) gets a globe and is never looked up.
* The flag `ui.site_icons` turns it off.
