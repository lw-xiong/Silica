// Point the download buttons at the visitor's platform.
//
// Every address on the page is already a working direct download, and all
// three ship in the markup, so with JavaScript off — or on a platform none of
// them matches — the pages stand as written. What detection does is narrow the
// offer:
//
//   - in a [data-dl-row], the matching [data-dl] button becomes the primary one
//     and the other two are hidden, because the Get it band further down the
//     page carries all three anyway;
//   - a lone [data-dl-main] button (the download page's hero, where the three
//     cards sit directly beneath it) is retargeted and relabelled instead;
//   - the matching .dl-grid card is marked [data-you], which the stylesheet
//     gives the accent border and a "Yours" chip.
(function () {
  var files = {
    linux:   { file: 'silica-linux-x86_64.tar.gz',  label: 'Download for Linux' },
    windows: { file: 'silica-windows-x86_64.zip',   label: 'Download for Windows' },
    macos:   { file: 'silica-macos-aarch64.tar.gz', label: 'Download for macOS' },
  };

  // userAgentData where it exists, the user-agent string otherwise. Android
  // and iOS deliberately match nothing: there is no build for either, and
  // offering one of the three desktop archives would be worse than offering none.
  function detect() {
    var ua = navigator.userAgent || '';
    var plat = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
    var s = (plat + ' ' + ua).toLowerCase();
    if (/android|iphone|ipad|ipod/.test(s)) return null;
    if (/win/.test(s)) return 'windows';
    if (/mac|darwin/.test(s)) return 'macos';
    if (/linux|x11|bsd/.test(s)) return 'linux';
    return null;
  }

  var me = detect();
  if (!me || !files[me]) return;

  // A row of per-platform buttons: keep the one that fits, drop the others.
  // The row's transitions are off while this happens — the script runs after
  // the first paint, and a button that fades from hairline to ink on load
  // reads as a glitch rather than as a transition.
  Array.prototype.forEach.call(document.querySelectorAll('[data-dl-row]'), function (row) {
    var mine = row.querySelector('[data-dl="' + me + '"]');
    if (!mine) return;
    row.classList.add('dl-switching');
    Array.prototype.forEach.call(row.querySelectorAll('[data-dl]'), function (btn) {
      if (btn === mine) return;
      btn.classList.remove('primary');
      btn.hidden = true;
    });
    mine.classList.add('primary');
    void row.offsetWidth;   // apply it before the transitions come back
    requestAnimationFrame(function () { row.classList.remove('dl-switching'); });
  });

  // A single button that stands for whichever platform the reader is on.
  var main = document.querySelector('[data-dl-main]');
  if (main) {
    main.href = main.href.replace(/[^/]+$/, files[me].file);
    main.textContent = files[me].label;
  }

  var card = document.querySelector('.dl-grid [data-platform="' + me + '"]');
  if (card) card.setAttribute('data-you', '');
})();
