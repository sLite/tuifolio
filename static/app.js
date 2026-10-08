function updateTransactionRequirements(form) {
  if (form.dataset.transactionId) return;
  const trade = ["Buy", "Sell"].includes(form.elements.kind.value);
  form.elements.quote_asset_id.required = trade || form.elements.quote_amount.value.trim() !== "";
  form.elements.quote_amount.required = trade;
  form.elements.fee_asset_id.required = form.elements.fee_amount.value.trim() !== "";
  form.elements.fee_amount.required = form.elements.fee_asset_id.value !== "";
}

function initializeForms(root) {
  root.querySelectorAll("[data-transaction-form]").forEach(updateTransactionRequirements);
}

document.addEventListener("change", event => {
  const input = event.target;
  const form = input.form;
  if (!form) return;
  if (input.hasAttribute("data-submit-on-change")) form.requestSubmit();
  if (!form.hasAttribute("data-transaction-form")) return;
  updateTransactionRequirements(form);
});

document.addEventListener("input", event => {
  const form = event.target.form;
  if (form?.hasAttribute("data-transaction-form")) updateTransactionRequirements(form);
});

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
