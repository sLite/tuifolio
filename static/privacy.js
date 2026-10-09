const privacyStorageKey = "tuifolio.privacy";

function restorePrivacyMode() {
  try {
    return localStorage.getItem(privacyStorageKey) === "true";
  } catch {
    return false;
  }
}

function updatePrivacyToggle() {
  const enabled = document.documentElement.classList.contains("privacy-mode");
  document.querySelectorAll("[data-privacy-toggle]").forEach(button => {
    button.hidden = false;
    button.setAttribute("aria-pressed", String(enabled));
    button.querySelector("[data-privacy-state]").textContent = enabled ? "On" : "Off";
  });
}

function setPrivacyMode(enabled) {
  document.documentElement.classList.toggle("privacy-mode", enabled);
  updatePrivacyToggle();
}

// Run in the head before content is painted, including on full-page reloads.
setPrivacyMode(restorePrivacyMode());

document.addEventListener("click", event => {
  if (!event.target.closest("[data-privacy-toggle]")) return;
  const enabled = !document.documentElement.classList.contains("privacy-mode");
  setPrivacyMode(enabled);
  try {
    localStorage.setItem(privacyStorageKey, String(enabled));
  } catch {
    // The toggle still works for this page when browser storage is unavailable.
  }
});

window.addEventListener("storage", event => {
  if (event.key === privacyStorageKey || event.key === null) {
    setPrivacyMode(restorePrivacyMode());
  }
});
document.addEventListener("DOMContentLoaded", updatePrivacyToggle);
document.addEventListener("htmx:afterSwap", updatePrivacyToggle);
