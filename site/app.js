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
