const searchTimers = new WeakMap();

function cancelSearchSubmission(form) {
  clearTimeout(searchTimers.get(form));
  searchTimers.delete(form);
}

function scheduleSearchSubmission(event) {
  const input = event.target;
  const form = input.form;
  if (!form?.hasAttribute("data-search-form") || input.type !== "search") return;
  cancelSearchSubmission(form);
  htmx.trigger(form, "htmx:abort");
  if (event.isComposing) return;
  searchTimers.set(form, setTimeout(() => {
    searchTimers.delete(form);
    if (form.isConnected) form.requestSubmit();
  }, 300));
}

function updateTransactionRequirements(form) {
  const trade = ["Buy", "Sell"].includes(form.elements.kind.value);
  form.elements.quote_asset_id.required = trade || form.elements.quote_amount.value.trim() !== "";
  form.elements.quote_amount.required = trade;
  form.elements.fee_asset_id.required = form.elements.fee_amount.value.trim() !== "";
  form.elements.fee_amount.required = form.elements.fee_asset_id.value !== "";
}

function updateIntrinsicValuation(form) {
  const valuation = form.querySelector("[data-intrinsic-valuation]");
  const applicable = ["Property", "Liability", "Custom"].includes(form.elements.kind.value);
  valuation.hidden = !applicable;
  valuation.disabled = !applicable;
}

function initializeForms(root) {
  root.querySelectorAll("[data-transaction-form]").forEach(updateTransactionRequirements);
  root.querySelectorAll("[data-asset-form]").forEach(updateIntrinsicValuation);
}

document.addEventListener("change", event => {
  const input = event.target;
  const form = input.form;
  if (!form) return;
  if (form.hasAttribute("data-search-form") && input.tagName === "SELECT") {
    cancelSearchSubmission(form);
    form.requestSubmit();
  }
  if (input.hasAttribute("data-submit-on-change")) form.requestSubmit();
  if (form.hasAttribute("data-asset-form") && input.name === "kind") {
    updateIntrinsicValuation(form);
  }
  if (!form.hasAttribute("data-transaction-form")) return;
  updateTransactionRequirements(form);
});

document.addEventListener("input", event => {
  scheduleSearchSubmission(event);
  const form = event.target.form;
  if (form?.hasAttribute("data-transaction-form")) updateTransactionRequirements(form);
});

document.addEventListener("compositionend", scheduleSearchSubmission);
document.addEventListener("submit", event => cancelSearchSubmission(event.target));

document.addEventListener("DOMContentLoaded", () => initializeForms(document));
document.addEventListener("htmx:load", event => initializeForms(event.detail.elt));
document.addEventListener("htmx:sendError", () => {
  const content = document.querySelector("#content");
  if (!content || content.querySelector("[data-network-error]")) return;
  const message = document.createElement("p");
  message.className = "notice error";
  message.dataset.networkError = "true";
  message.setAttribute("role", "alert");
  message.textContent = "Could not reach Tuifolio. Check that the server is running, then retry.";
  content.prepend(message);
});
