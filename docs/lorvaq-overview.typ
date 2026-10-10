#import "@preview/fletcher:0.5.8" as fletcher: diagram, node, edge
#set document(title: "lorvaq: живые схемы real-time мониторинга", author: "Alimzhan Kangerey")
#set page(
  paper: "a4",
  margin: (top: 1.7cm, bottom: 1.5cm, x: 1.7cm),
  numbering: "1",
  header: context {
    if counter(page).get().first() > 1 [
      #set text(size: 9pt, fill: rgb("#495057"), font: "PT Sans")
      #grid(columns: (1fr, 1fr), align(left)[lorvaq · обзор продукта], align(right)[10.10.2026])
      #line(length: 100%, stroke: 0.5pt + rgb("#ced4da"))
    ]
  },
)
#set text(font: "PT Sans", size: 9.5pt, fill: rgb("#1a1a1a"), lang: "ru")
#set par(leading: 0.55em, spacing: 0.85em, justify: false)
#set table(stroke: 0.5pt + rgb("#dee2e6"), inset: 4.5pt)
#show table: set text(size: 8.5pt)
#show raw: set text(size: 0.92em)
#set list(spacing: 0.5em)
#show heading.where(level: 1): it => [
  #v(4pt)
  #block(text(size: 14pt, weight: "bold", fill: rgb("#12151c"))[#it.body])
  #line(length: 100%, stroke: 1.5pt + rgb("#12151c"))
  #v(2pt)
]

#let hd(b) = text(weight: "bold", size: 8.5pt)[#b]
#let sub(title) = text(weight: "bold", size: 8.5pt, fill: rgb("#40464f"), tracking: 0.5pt)[#upper(title)]
#let pill(fill, fg, body) = box(fill: fill, inset: (x: 5pt, y: 2.5pt), radius: 3pt)[#text(fill: fg, size: 7.5pt, weight: "bold")[#body]]
#let tstat(s) = if s == "done" { pill(rgb("#2f9e44"), white)[СДЕЛАНО] } else if s == "wip" { pill(rgb("#1c7ed6"), white)[В РАБОТЕ] } else { pill(rgb("#868e96"), white)[НЕ НАЧАТО] }
#let task(id, title, status: "todo", goal: none, decisions: none, done: none) = block(
  fill: rgb("#f8f9fa"), stroke: (left: 3pt + rgb("#1c7ed6")), inset: 7pt, radius: 4pt, width: 100%, breakable: false, above: 6pt, below: 0pt, height: auto,
)[
  #tstat(status) #h(5pt) #text(weight: "bold", size: 10.5pt)[#id #title] \
  #v(1pt)
  #sub("что построить") #h(4pt) #goal \
  #if decisions != none [#sub("решить") #h(4pt) #decisions \ ]
  #sub("готово, когда") #h(4pt) #done
]

// ---- scene mock helpers ----
#let c-blue = rgb("#2a78d6")
#let c-violet = rgb("#6250d6")
#let c-orange = rgb("#eb6834")
#let c-aqua = rgb("#1baf7a")
#let c-amber = rgb("#fab219")
#let c-red = rgb("#e03131")
#let tok(c, ring: none) = box(baseline: 1.5pt, circle(radius: 3.4pt, fill: c, stroke: if ring == none { 1pt + white } else { 1.6pt + ring }))
#let bar(pct, col) = box(width: 100%, height: 4pt, fill: rgb("#e9ecef"), radius: 2pt)[#box(width: pct * 1%, height: 4pt, fill: col, radius: 2pt)]
#let slot(label, pct, col, svc: false) = block(width: 100%, inset: (x: 4pt, y: 3pt), radius: 3pt, stroke: 0.5pt + rgb("#ced4da"), fill: white, below: 3pt)[
  #text(size: 7.5pt)[#label] #h(1fr) #if svc { box(inset: (x: 2pt, y: 0.5pt), stroke: (dash: "dashed", paint: rgb("#868e96"), thickness: 0.5pt), radius: 2pt)[#text(size: 6.5pt, fill: rgb("#495057"))[svc]] } \
  #bar(pct, col)
]
#let free-slot = block(width: 100%, inset: (x: 4pt, y: 3pt), radius: 3pt, stroke: (dash: "dashed", paint: rgb("#adb5bd"), thickness: 0.5pt), below: 3pt)[#text(size: 7.5pt, fill: rgb("#868e96"))[свободен] \ #v(4pt)]
#let pbox(c, name) = box(width: 30mm, inset: 5pt, radius: 4pt, stroke: 0.6pt + rgb("#ced4da"), fill: rgb("#f8f9fa"))[#tok(c) #h(3pt) #text(size: 8pt)[#name]]
#let elab = (label-size: 7.5pt, label-fill: white)

// ================================================================
#align(center)[
  #text(size: 22pt, weight: "bold")[lorvaq]
  #v(1pt)
  #text(size: 12pt, fill: rgb("#495057"))[Живые схемы real-time мониторинга · глаза для серверов маленькой студии]
  #v(1pt)
  #text(size: 9pt, fill: rgb("#868e96"))[Обзор продукта · 10.10.2026 · подробный дизайн: `docs/design/2026-10-10-core-and-ci-scene.md`]
]
#v(2pt)

#text(size: 8.5pt, fill: rgb("#495057"))[*Как читать.* Раздел 1 — что такое lorvaq, как выглядит первая сцена и откуда берутся данные. Раздел 2 — принятые решения. Раздел 3 — задачи-пакеты первого проекта «ядро + сцена CI» в порядке зависимости. Пара к kalysta: kalysta — руки (восстановить, переключить, впустить), lorvaq — глаза (увидеть, что происходит и почему).]

= 1. Картина

*Что это.* Самостоятельное self-hosted приложение, которое показывает систему не числами на осях, как Grafana, а *вещами в местах, которые движутся*: джоб едет из очереди в слот раннера, бэкап — с диска в S3, запрос — через сервисы. Цель — с одного взгляда видеть не «сколько», а «почему и где»: этот джоб ждёт, потому что занят лимит раннера; этот застрял — нет раннера с его тегом; этот слот упал по памяти.

*Как живёт.* Один docker-образ со своим интерфейсом. Сцены встроены и включаются в настройках — каждый берёт нужные. GitLab подключается кнопкой Authorize, хосты — командой с join-токеном; через env только адрес базы и ключ шифрования. Только чтение: lorvaq ничего не запускает и не меняет. Встраивающая система (админка студии) только запускает образ и ставит ссылку.

*Модель мира.* Любая сцена описывается четырьмя понятиями — ядро хранит их и не знает, откуда пришли данные:

#table(
  columns: (auto, 1fr, 1fr, 1fr),
  hd(""), hd("Сцена CI (первая)"), hd("Сцена бэкапов"), hd("Сцена трафика"),
  [*Места*], [проекты, очередь, хосты, процессы раннеров, слоты, выходы], [серверы, базы, диск, S3-бакеты], [вход, сервисы, базы],
  [*Сущности*], [джобы], [прогоны бэкапа], [запросы],
  [*Перемещения*], [очередь → слот → registry / prod], [база → диск → S3], [вход → сервис → база],
  [*Уровни*], [память джоба / лимит, диск хоста], [размер бакета, свежесть], [латентность, ошибки],
)

#v(2pt)
*Как выглядит сцена CI* (симуляция, данные выдуманы; вживую токены едут, слоты наполняются, OOM вспыхивает):

#let col(w, body) = box(width: w, body)
#let stack-box(w, title, body, stroke: 0.6pt + rgb("#ced4da"), fill: rgb("#f8f9fa")) = box(width: w, inset: 6pt, radius: 4pt, stroke: stroke, fill: fill)[#title #v(2pt) #body]
#align(center, grid(
  columns: (auto, 7mm, auto, 7mm, auto, 7mm, auto),
  align: horizon + center,
col(30mm)[
    #text(size: 7.5pt, fill: rgb("#495057"))[проекты] \ #v(1pt)
    #pbox(c-blue)[backend] #v(3pt)
    #pbox(c-violet)[frontend] #v(3pt)
    #pbox(c-orange)[mobile] #v(3pt)
    #pbox(c-aqua)[legacy]
  ],
text(size: 16pt, fill: rgb("#868e96"))[→],
col(32mm)[
    #stack-box(32mm, text(size: 8pt, weight: "bold")[очередь GitLab])[
      #tok(c-blue) #tok(c-violet) #tok(c-orange, ring: c-amber) \ #v(1pt)
      #text(size: 6.8pt, fill: rgb("#495057"))[жёлтая обводка — ждёт лимит раннера при свободном слоте]
    ]
    #v(5pt)
    #stack-box(32mm, text(size: 8pt, weight: "bold", fill: rgb("#c92a2a"))[нет раннера], stroke: (dash: "dashed", paint: c-red, thickness: 0.7pt), fill: rgb("#fff5f5"))[
      #tok(c-aqua) #tok(c-aqua) #h(2pt) #text(size: 6.8pt, fill: rgb("#c92a2a"))[тег `docker`, ждут 38 мин]
    ]
  ],
text(size: 16pt, fill: rgb("#868e96"))[→],
box(width: 56mm, inset: 6pt, radius: 5pt, stroke: 0.9pt + rgb("#1c7ed6"), fill: rgb("#f1f8ff"))[
    #text(size: 9pt, weight: "bold")[хост раннеров] #h(1fr) #text(size: 7pt, fill: rgb("#2f9e44"))[агент онлайн] \
    #v(2pt)
    #text(size: 7pt, fill: rgb("#495057"))[процесс shared · 5 слотов · cgroup 5.9 / 8 ГБ]
    #slot([backend · test · 2.1/3 ГБ], 70, c-blue)
    #slot([frontend · build · 1.2/3 ГБ], 40, c-violet, svc: true)
    #slot([mobile · e2e · 1.85/2 ГБ], 92, c-amber, svc: true)
    #free-slot
    #block(width: 100%, inset: (x: 4pt, y: 3pt), radius: 3pt, stroke: 0.9pt + c-red, fill: rgb("#fff5f5"), below: 3pt)[#text(size: 7.5pt, fill: rgb("#c92a2a"))[*OOM* · backend · lint · 3/3 ГБ] \ #bar(100, c-red)]
    #line(length: 100%, stroke: 1.4pt + rgb("#495057"))
    #text(size: 7pt, fill: rgb("#495057"))[процесс deploy · отдельный docker-демон]
    #free-slot
    #text(size: 7pt, fill: rgb("#495057"))[диск данных docker] #h(1fr) #text(size: 7pt, weight: "bold")[71%] \ #bar(71, c-amber)
  ],
text(size: 16pt, fill: rgb("#868e96"))[→],
col(30mm)[
    #text(size: 7.5pt, fill: rgb("#495057"))[выходы] \ #v(1pt)
    #pbox(rgb("#868e96"))[registry] \ #text(size: 6.8pt, fill: rgb("#495057"))[успешный build улетает сюда] #v(4pt)
    #pbox(rgb("#868e96"))[prod] \ #text(size: 6.8pt, fill: rgb("#495057"))[успешный deploy]
  ],
))

*Откуда данные.* Сервер опрашивает GitLab и принимает отчёты агентов, сводит их в мир сцены, пишет историю и отдаёт браузеру снимок и поток событий.

#align(center, diagram(
  spacing: (14mm, 4mm),
  node-stroke: 0.6pt + rgb("#ced4da"), node-corner-radius: 4pt, node-inset: 6pt, node-fill: rgb("#f8f9fa"),
  node((0, 0), [*GitLab* \ #text(size: 7.5pt)[GraphQL · `read_api` · опрос 3 с]], name: <gl>),
  node((0, 1), [*agent* на хосте раннеров \ #text(size: 7.5pt)[docker API, только чтение · исходящий WebSocket]], name: <ag>),
  node((1, 0.5), [*lorvaq server* \ #text(size: 7.5pt)[источники → редьюсер → мир → события]], name: <srv>, fill: rgb("#f1f8ff"), stroke: 0.9pt + rgb("#1c7ed6")),
  node((2, 0), [*Postgres* \ #text(size: 7.5pt)[история 14 дней]], name: <pg>),
  node((2, 1), [*браузер* \ #text(size: 7.5pt)[снимок + SSE · SVG-анимация]], name: <br>),
  edge(<gl>, <srv>, "-|>"),
  edge(<ag>, <srv>, "-|>"),
  edge(<srv>, <pg>, "-|>"),
  edge(<srv>, <br>, "-|>"),
))

#pagebreak()

= 2. Что решено

#v(2pt)

#table(
  columns: (auto, 1fr),
  hd("Решение"), hd("Почему"),
  [Публичный репо `alfinkly/lorvaq`, MIT], [продукт автора; любая студия, включая первую, пользуется им по лицензии как внешней зависимостью],
  [Rust, один бинарник: `server` / `agent`], [как kalysta: статическая сборка, агент почти не ест память рядом с раннерами],
  [Docker-образ, Postgres (`sqlx`)], [запуск одной командой (compose в репо); историю можно читать и из Grafana],
  [Сцены встроены, включаются в настройках], [проще плагинов; внешние сцены — когда их начнут писать другие люди],
  [Настройка из интерфейса], [GitLab — OAuth Authorize или токен по готовой ссылке; хосты — команда с join-токеном],
  [Один администратор + режимы зрителей], [`open` / `proxy` (за forward-auth студии) / `admin`; первый пароль — по коду из лога],
  [Ничего о конкретной студии в репо], [ни хостов, ни доменов, ни скриншотов с реальными данными — только `--demo`; `gitleaks` в CI],
)

= 3. Задачи первого проекта: ядро + сцена CI

#grid(columns: (1fr, 1fr), column-gutter: 8pt, row-gutter: 0pt,
task("Л0", "Дизайн", status: "done",
  goal: [модель мира, компоненты, сцена CI, доступ, API, хранение, деградация, тесты — в `docs/design/`.],
  done: [дизайн прочитан и принят владельцем; дальше — план реализации по шагам.]),
task("Л1", "Разведка на живом раннере", status: "todo",
  goal: [проверить docker-метки gitlab-runner, формат имён контейнеров (`concurrent-N`), события `die`/`OOMKilled`, `statvfs` в rootless-демоне.],
  decisions: [на чём держится связка «джоб ↔ слот ↔ память», если меток не хватит.],
  done: [записка с фактами (без данных студии) и синтетические фикстуры для тестов агента.]),
task("Л2", "Ядро: мир, шина, снимок и SSE, история", status: "todo",
  goal: [типы мира и событий, применение, снимок + события = тот же мир, SSE с `Last-Event-ID` и `410`, Postgres с миграциями и чисткой, живая схема без базы.],
  done: [тесты ядра зелёные; сервер в `--demo` отдаёт снимок и поток событий.]),
task("Л3", "Доступ и настройки", status: "todo",
  goal: [первый запуск по коду, администратор, сессии, режимы зрителей, шифрование секретов, страницы настроек.],
  done: [без входа администратора настройки недоступны в любом режиме; секреты не возвращаются API.]),
task("Л4", "Источник GitLab", status: "todo",
  goal: [мастер подключения (OAuth + PKCE с обновлением токена или токен по ссылке), проверка прав, опрос GraphQL, баннер «данные устарели».],
  done: [подключение за минуту из интерфейса; отзыв токена виден в настройках.]),
task("Л5", "Агент", status: "todo",
  goal: [`lorvaq agent --join`: слоты, память джоба и сервисов, диск, OOM; исходящий WebSocket с переподключением; «Добавить хост» в интерфейсе.],
  done: [хост появляется сам после `docker run`; офлайн-агент — серый хост, а не ошибка.]),
task("Л6", "Сцена CI и движок отрисовки", status: "todo",
  goal: [редьюсер (очередь, «нет раннера», «ждёт лимит», OOM, выходы) и раскладка на примитивах: контейнер, слот, токен, перелёт, бак, вспышка.],
  done: [на реальном GitLab джобы видны с задержкой не больше 5 с; на `--demo` — так же, как на картинке выше.]),
task("Л7", "Релиз", status: "todo",
  goal: [GitHub Actions (тесты, clippy, образ в ghcr по тегу), `compose.yml` и `compose.demo.yml`, README на английском с демо-гифкой.],
  done: [`docker compose -f compose.demo.yml up` у постороннего человека показывает живую схему.])
)
