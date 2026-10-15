(function () {
  var platform = ((navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || navigator.userAgent || "").toLowerCase();
  var os = /mac|iphone|ipad/.test(platform) ? "mac" : /win/.test(platform) ? "windows" : /linux|x11/.test(platform) ? "linux" : "";
  var names = { mac: "macOS", windows: "Windows", linux: "Linux" };
  var button = document.getElementById("primary-download");
  if (!os || !button) return;
  if (/iphone|ipad|android/.test(platform)) return;
  button.textContent = "Download for " + names[os];
  var rows = document.querySelectorAll('.spec tr[data-os="' + os + '"]');
  for (var i = 0; i < rows.length; i++) rows[i].classList.add("is-yours");
})();
