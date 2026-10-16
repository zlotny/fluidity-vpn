(function () {
  var still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  var canvas = document.getElementById("starfield");
  var ctx = canvas && canvas.getContext("2d");
  var layers = [];
  var twinklers = [];
  var comets = [];
  var width = 0;
  var height = 0;
  var dpr = 1;
  var scroll = window.scrollY;
  var drawn = -1;

  function layer(count, minR, maxR, alpha, parallax) {
    var c = document.createElement("canvas");
    c.width = width * dpr;
    c.height = height * dpr;
    var g = c.getContext("2d");
    g.scale(dpr, dpr);
    for (var i = 0; i < count; i++) {
      var tint = Math.random() < 0.18;
      g.globalAlpha = alpha * (0.5 + Math.random() * 0.5);
      g.fillStyle = tint ? "hsl(" + (200 + Math.random() * 60) + ",90%,82%)" : "#fff";
      g.beginPath();
      g.arc(Math.random() * width, Math.random() * height, minR + Math.random() * (maxR - minR), 0, Math.PI * 2);
      g.fill();
    }
    return { canvas: c, parallax: parallax };
  }

  function resize() {
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    width = window.innerWidth;
    height = window.innerHeight;
    canvas.width = width * dpr;
    canvas.height = height * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    var area = (width * height) / 10000;
    layers = [
      layer(Math.round(area * 1.6), 0.3, 0.6, 0.55, 0.04),
      layer(Math.round(area * 0.7), 0.5, 0.9, 0.75, 0.1),
      layer(Math.round(area * 0.18), 0.9, 1.4, 0.9, 0.18)
    ];
    twinklers = [];
    for (var i = 0; i < Math.round(area * 0.12); i++) {
      twinklers.push({
        x: Math.random() * width,
        y: Math.random() * height,
        r: 0.8 + Math.random() * 0.9,
        phase: Math.random() * Math.PI * 2,
        speed: 0.6 + Math.random() * 1.4
      });
    }
    drawn = -1;
    draw(0);
  }

  function wrap(v) {
    v %= height;
    return v < 0 ? v + height : v;
  }

  function draw(t) {
    ctx.clearRect(0, 0, width, height);
    for (var i = 0; i < layers.length; i++) {
      var off = wrap(-scroll * layers[i].parallax);
      ctx.drawImage(layers[i].canvas, 0, off, width, height);
      ctx.drawImage(layers[i].canvas, 0, off - height, width, height);
    }
    ctx.fillStyle = "#fff";
    for (var j = 0; j < twinklers.length; j++) {
      var s = twinklers[j];
      ctx.globalAlpha = still ? 0.6 : 0.15 + 0.75 * Math.abs(Math.sin(t * 0.001 * s.speed + s.phase));
      ctx.beginPath();
      ctx.arc(s.x, wrap(s.y - scroll * 0.12), s.r, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.lineWidth = 1.4;
    for (var k = comets.length - 1; k >= 0; k--) {
      var c = comets[k];
      var tx = c.x - c.vx * 14;
      var ty = c.y - c.vy * 14;
      var g = ctx.createLinearGradient(c.x, c.y, tx, ty);
      g.addColorStop(0, "rgba(186,230,253," + c.life + ")");
      g.addColorStop(1, "rgba(186,230,253,0)");
      ctx.globalAlpha = 1;
      ctx.strokeStyle = g;
      ctx.beginPath();
      ctx.moveTo(c.x, c.y);
      ctx.lineTo(tx, ty);
      ctx.stroke();
      c.x += c.vx;
      c.y += c.vy;
      c.life -= 0.014;
      if (c.life <= 0) comets.splice(k, 1);
    }
    ctx.globalAlpha = 1;
  }

  var last = 0;
  function frame(t) {
    requestAnimationFrame(frame);
    if (document.hidden || t - last < 33) return;
    last = t;
    if (Math.random() < 0.004) {
      comets.push({
        x: width * (0.3 + Math.random() * 0.7),
        y: height * Math.random() * 0.4,
        vx: -(7 + Math.random() * 4),
        vy: 3 + Math.random() * 2,
        life: 1
      });
    }
    draw(t);
  }

  if (ctx) {
    resize();
    var pending;
    window.addEventListener("resize", function () {
      clearTimeout(pending);
      pending = setTimeout(resize, 150);
    });
    if (!still) requestAnimationFrame(frame);
  }

  var nav = document.querySelector(".nav");
  function onScroll() {
    scroll = window.scrollY;
    if (nav) nav.classList.toggle("is-scrolled", scroll > 40);
    if (still && ctx && drawn !== scroll) {
      drawn = scroll;
      draw(0);
    }
  }
  window.addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  var items = document.querySelectorAll(".reveal");
  if ("IntersectionObserver" in window && !still) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) {
          e.target.classList.add("is-in");
          io.unobserve(e.target);
        }
      });
    }, { rootMargin: "0px 0px -8% 0px" });
    items.forEach(function (el) { io.observe(el); });
  } else {
    items.forEach(function (el) { el.classList.add("is-in"); });
  }

  document.querySelectorAll(".card").forEach(function (card) {
    card.addEventListener("pointermove", function (e) {
      var r = card.getBoundingClientRect();
      card.style.setProperty("--mx", e.clientX - r.left + "px");
      card.style.setProperty("--my", e.clientY - r.top + "px");
    });
  });

  var tabs = document.querySelectorAll(".shot-tabs button");
  var shots = document.querySelectorAll(".shot-stage img");
  tabs.forEach(function (tab) {
    tab.addEventListener("click", function () {
      var name = tab.getAttribute("data-shot");
      tabs.forEach(function (t) { t.setAttribute("aria-selected", t === tab ? "true" : "false"); });
      shots.forEach(function (img) { img.classList.toggle("is-on", img.getAttribute("data-shot") === name); });
    });
  });
})();
