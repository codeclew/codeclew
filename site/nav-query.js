"use strict";

const panels = [...document.querySelectorAll(".claim-panel")];
const selector = document.querySelector("#claim-select");
const graph = document.querySelector(".flow-graph");
const levelButtons = [...document.querySelectorAll("[data-level]")];
let level = "overview";

function selectClaim(id, updateLocation = false) {
  const chosen = panels.find(panel => panel.id === `claim-${id}`);
  if (!chosen) return;
  panels.forEach(panel => { panel.hidden = panel !== chosen; });
  selector.value = id;
  document.querySelectorAll(".flow-graph .node").forEach(node => {
    node.classList.toggle("is-selected", node.id === `node-${id}`);
  });
  document.querySelectorAll(".flow-graph [data-claim]").forEach(link => {
    if (link.dataset.claim === id) link.setAttribute("aria-current", "true");
    else link.removeAttribute("aria-current");
  });
  chosen.querySelector(".claim-evidence").open = level === "evidence";
  if (updateLocation) history.replaceState(null, "", `#claim-${id}`);
}

function readHash() {
  const id = location.hash.replace(/^#claim-/, "");
  selectClaim(panels.some(panel => panel.id === `claim-${id}`) ? id : "entry");
}

graph.addEventListener("click", event => {
  const link = event.target.closest("[data-claim]");
  if (!link) return;
  event.preventDefault();
  selectClaim(link.dataset.claim, true);
  if (matchMedia("(max-width: 850px)").matches) {
    document.querySelector(".claim-reader").scrollIntoView({ block: "start" });
  }
});
graph.addEventListener("keydown", event => {
  if (event.key !== " ") return;
  const link = event.target.closest("[data-claim]");
  if (link) { event.preventDefault(); link.dispatchEvent(new MouseEvent("click", { bubbles: true })); }
});
selector.addEventListener("change", () => selectClaim(selector.value, true));
levelButtons.forEach(button => button.addEventListener("click", () => {
  level = button.dataset.level;
  levelButtons.forEach(item => item.setAttribute("aria-pressed", String(item === button)));
  selectClaim(selector.value);
}));
window.addEventListener("hashchange", readHash);
readHash();
