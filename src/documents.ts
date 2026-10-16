import "./documents.css";

/** A row of `GET /api/documents`. */
type DocumentSummary = {
  id: string;
  title: string | null;
  language: string | null;
  updated_at: string | null;
  preview: string;
};

// Follow the editor's dark mode setting (stored by use-local-storage-state).
try {
  if (JSON.parse(localStorage.getItem("darkMode") ?? "false") === true) {
    document.documentElement.classList.add("dark");
  }
} catch {
  // Unavailable storage: keep the light theme.
}

const filter = document.querySelector<HTMLInputElement>("#filter")!;
const status = document.querySelector<HTMLParagraphElement>("#status")!;
const table = document.querySelector<HTMLTableElement>("#documents")!;
const tbody = table.querySelector("tbody")!;

let documents: DocumentSummary[] = [];

const dateFormat = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
  timeStyle: "short",
});

function firstLine(text: string): string {
  return text.trim().split("\n")[0].trim();
}

/** Sends a change to the owner API; returns the parsed JSON response. */
async function api(method: string, path: string, body: unknown): Promise<any> {
  const response = await fetch(path, {
    method,
    credentials: "same-origin",
    headers: { "Content-Type": "application/json", Accept: "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}`);
  }
  return response.json();
}

function button(label: string, onClick: () => void): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = label;
  b.addEventListener("click", onClick);
  return b;
}

async function rename(doc: DocumentSummary) {
  const title = prompt(`Title for ${doc.id} (empty to clear)`, doc.title ?? "");
  if (title === null) return;
  try {
    const result = await api("PUT", `api/documents/${doc.id}/title`, { title });
    doc.title = result.title;
    render();
  } catch (error) {
    alert(`Could not rename: ${error}`);
  }
}

function row(doc: DocumentSummary): HTMLTableRowElement {
  const tr = document.createElement("tr");

  const title = document.createElement("td");
  const link = document.createElement("a");
  link.href = `/#${doc.id}`;
  if (doc.title) {
    link.textContent = doc.title;
  } else {
    link.textContent = firstLine(doc.preview) || "(empty)";
    link.className = "untitled";
  }
  const id = document.createElement("span");
  id.className = "id";
  id.textContent = doc.id;
  title.append(link, id);

  const language = document.createElement("td");
  language.textContent = doc.language ?? "";

  const updated = document.createElement("td");
  if (doc.updated_at) {
    const date = new Date(doc.updated_at);
    updated.textContent = dateFormat.format(date);
    updated.title = date.toISOString();
  } else {
    updated.textContent = "–";
    updated.title = "Not saved since dates were recorded";
  }

  const actions = document.createElement("td");
  actions.className = "actions";
  actions.append(button("Rename", () => rename(doc)));

  tr.append(title, language, updated, actions);
  return tr;
}

function matches(doc: DocumentSummary, query: string): boolean {
  return [doc.id, doc.title, doc.language, doc.preview].some((field) =>
    field?.toLowerCase().includes(query),
  );
}

function render() {
  const query = filter.value.trim().toLowerCase();
  const shown = query
    ? documents.filter((doc) => matches(doc, query))
    : documents;
  tbody.replaceChildren(...shown.map(row));
  table.hidden = shown.length === 0;
  status.textContent =
    shown.length === documents.length
      ? `${documents.length} documents`
      : `${shown.length} of ${documents.length} documents`;
}

async function load() {
  try {
    const response = await fetch("api/documents", {
      credentials: "same-origin",
      headers: { Accept: "application/json" },
    });
    if (!response.ok) {
      throw new Error(`${response.status} ${response.statusText}`);
    }
    documents = await response.json();
    render();
  } catch (error) {
    status.textContent = `Could not load the documents: ${error}`;
  }
}

filter.addEventListener("input", render);
load();
