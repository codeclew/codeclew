const button = document.querySelector("#copy-command");
const command = document.querySelector("#install-command");

if (button && command) {
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(command.textContent.trim());
      button.querySelector("span").textContent = "Copied";
      window.setTimeout(() => {
        button.querySelector("span").textContent = "Copy";
      }, 1800);
    } catch (_error) {
      const selection = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(command);
      selection.removeAllRanges();
      selection.addRange(range);
    }
  });
}

// Shared progressive enhancement. Navigation itself is present in every HTML page.
(() => {
  const menu = document.querySelector(".site-menu");
  const sidebar = document.querySelector(".sidebar-disclosure");
  const narrow = matchMedia("(max-width: 850px)");
  const syncSidebar = () => { if (sidebar) sidebar.open = !narrow.matches; };
  syncSidebar();
  narrow.addEventListener("change", syncSidebar);
  document.addEventListener("click", event => {
    if (menu && !menu.contains(event.target)) menu.open = false;
  });
  document.addEventListener("keydown", event => {
    if (event.key === "Escape" && menu?.open) {
      menu.open = false;
      menu.querySelector("summary").focus();
    }
  });

  const dialog = document.querySelector("#site-search");
  const trigger = document.querySelector(".search-trigger");
  if (!dialog || !trigger || typeof dialog.showModal !== "function") return;
  const input = dialog.querySelector("input");
  const results = dialog.querySelector("#search-results");
  const status = dialog.querySelector("#search-status");
  let index;
  let pending;
  trigger.hidden = false;

  function renderResults() {
    const words = input.value.toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
    const matches = index.filter(entry => {
      const text = `${entry.title} ${entry.description} ${entry.group}`.toLocaleLowerCase();
      return words.every(word => text.includes(word));
    }).sort((a, b) => {
      if (!words.length) return Number(a.url.includes("#")) - Number(b.url.includes("#"));
      const score = entry => words.filter(word => entry.title.toLocaleLowerCase().includes(word)).length;
      return score(b) - score(a);
    });
    results.replaceChildren();
    for (const entry of matches.slice(0, 18)) {
      const link = document.createElement("a");
      link.className = "search-result";
      link.href = entry.url;
      const title = document.createElement("strong");
      title.textContent = entry.title;
      const description = document.createElement("small");
      description.textContent = `${entry.group} · ${entry.description}`;
      link.append(title, description);
      // Same-page fragment navigation should dismiss the modal too.
      link.addEventListener("click", () => dialog.close());
      results.append(link);
    }
    status.textContent = matches.length
      ? `${matches.length} results${matches.length > 18 ? " · showing the first 18" : ""}. Use Tab or arrow keys to explore.`
      : "No matching pages. Try a shorter phrase, “Spring”, or “example”.";
  }

  async function openSearch() {
    if (dialog.open) return;
    if (menu) menu.open = false;
    dialog.showModal();
    input.focus();
    status.textContent = "Loading pages and sections…";
    try {
      pending ??= fetch("./search-index.json").then(response => {
        if (!response.ok) throw new Error("Search index unavailable");
        return response.json();
      });
      index = await pending;
      renderResults();
    } catch (_error) {
      pending = undefined;
      status.textContent = "Search is unavailable. Close this window and use Explore to browse every page.";
    }
  }
  trigger.addEventListener("click", openSearch);
  dialog.querySelector("[data-close-search]").addEventListener("click", () => dialog.close());
  dialog.addEventListener("close", () => trigger.focus());
  dialog.addEventListener("click", event => {
    const box = dialog.getBoundingClientRect();
    if (event.target === dialog && (event.clientX < box.left || event.clientX > box.right || event.clientY < box.top || event.clientY > box.bottom)) dialog.close();
  });
  input.addEventListener("input", () => { if (index) renderResults(); });
  dialog.addEventListener("keydown", event => {
    const links = [...results.querySelectorAll("a")];
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      if (!links.length) return;
      event.preventDefault();
      const current = links.indexOf(document.activeElement);
      const next = current + (event.key === "ArrowDown" ? 1 : -1);
      links[(next + links.length) % links.length].focus();
    } else if (event.key === "Enter" && document.activeElement === input && links.length) {
      event.preventDefault();
      links[0].click();
    }
  });
  document.addEventListener("keydown", event => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      openSearch();
    }
  });
})();
