(function () {
  var platform = ((navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || navigator.userAgent || "").toLowerCase();
  if (/iphone|ipad|android/.test(platform)) return;
  var os = /mac/.test(platform) ? "mac" : /win/.test(platform) ? "windows" : /linux|x11/.test(platform) ? "linux" : "";
  var names = { mac: "macOS", windows: "Windows", linux: "Linux" };
  if (!os) return;
  var label = document.querySelector("#primary-download span");
  if (label) label.textContent = "Download for " + names[os];
  var card = document.querySelector('.platform[data-os="' + os + '"]');
  if (card) card.classList.add("is-yours");
})();
