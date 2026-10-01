// 報價單版面。
//
// 資料由 Rust 以 sys.inputs 注入，此處不做任何金額運算——
// 所有數字都是伺服器端算好的字串，照印即可。進位規則只有一份實作。

#let d = json(bytes(sys.inputs.data))

#set document(title: d.quote_no + " 報價單", author: d.company.name)
#set page(
  paper: "a4",
  margin: (x: 18mm, top: 16mm, bottom: 20mm),
  footer: context [
    #set text(8pt, fill: rgb("#8a9aa6"))
    #d.quote_no v#d.version
    #h(1fr)
    #counter(page).display("1 / 1", both: true)
  ],
)
#set text(font: d.font_stack, size: 9.5pt, lang: "zh", region: "tw")
#show heading: set text(fill: rgb("#1f3a4d"))

#let ink = rgb("#1f3a4d")
#let copper = rgb("#b87333")
#let line-colour = rgb("#dde4e8")
#let muted = rgb("#5f7181")

// ---------- 抬頭 ----------

#grid(
  columns: (auto, 1fr),
  column-gutter: 10pt,
  align: (horizon, horizon),
  if d.company.logo_svg != "" {
    image(bytes(d.company.logo_svg), format: "svg", width: 15mm)
  } else { none },
  [
    #text(15pt, weight: 700, fill: ink)[#d.company.name]
    #linebreak()
    #text(8pt, fill: muted)[
      #if d.company.tax_id != "" [統一編號 #d.company.tax_id #h(6pt)]
      #if d.company.phone != "" [電話 #d.company.phone #h(6pt)]
      #if d.company.fax != "" [傳真 #d.company.fax]
      #if d.company.address != "" [#linebreak() #d.company.address]
    ]
  ],
)

#v(6pt)
#line(length: 100%, stroke: 1pt + ink)
#v(10pt)

// ---------- 單據資訊 ----------

#grid(
  columns: (1fr, auto),
  [
    #text(16pt, weight: 700, fill: ink)[報 價 單]
  ],
  [
    #set text(8.5pt)
    #table(
      columns: (auto, auto),
      stroke: none,
      inset: (x: 0pt, y: 2pt),
      align: (right, left),
      text(fill: muted)[單號], [#h(6pt) #d.quote_no],
      text(fill: muted)[版本], [#h(6pt) 第 #d.version 版],
      text(fill: muted)[日期], [#h(6pt) #d.issued_on],
      ..if d.valid_until != "" {
        (text(fill: muted)[有效期限], [#h(6pt) #d.valid_until])
      } else { () },
    )
  ],
)

#v(8pt)

#block(
  fill: rgb("#f4f6f7"),
  inset: 9pt,
  radius: 3pt,
  width: 100%,
  [
    #set text(9pt)
    #text(fill: muted)[客戶] #h(5pt) #text(weight: 600)[#d.customer.name]
    #if d.customer.tax_id != "" [#h(10pt) #text(fill: muted)[統編] #h(4pt) #d.customer.tax_id]
    #linebreak()
    #text(fill: muted)[案場] #h(5pt) #d.project.name
    #if d.project.site_address != "" [#h(10pt) #text(fill: muted, size: 8.5pt)[#d.project.site_address]]
  ],
)

#v(10pt)

// ---------- 明細 ----------

#table(
  columns: (auto, 1fr, auto, auto, auto, auto, auto),
  stroke: none,
  inset: (x: 5pt, y: 6pt),
  align: (center, left, left, center, right, right, right),
  fill: (_, row) => if row == 0 { rgb("#f4f6f7") } else { none },

  table.header(
    ..([項次], [項目], [規格], [單位], [數量], [單價], [小計]).map(c =>
      text(8pt, weight: 600, fill: muted)[#c]
    )
  ),
  table.hline(stroke: 0.6pt + ink),

  ..d.items.map(it => (
    text(8.5pt, fill: muted)[#it.line_no],
    [#it.name],
    text(8.5pt, fill: muted)[#it.spec],
    [#it.unit],
    [#it.quantity],
    [#it.unit_price],
    text(weight: 600)[#it.amount],
  )).flatten(),

  table.hline(stroke: 0.6pt + line-colour),
)

#v(10pt)

// ---------- 合計 ----------

#align(right)[
  #table(
    columns: (auto, 72pt),
    stroke: none,
    inset: (x: 6pt, y: 3pt),
    align: (right, right),

    text(fill: muted)[未稅合計], [#d.subtotal],
    text(fill: muted)[營業稅 #d.tax_percent], [#d.tax_amount],
    table.hline(stroke: 0.6pt + ink),
    text(weight: 700)[含稅總計], text(weight: 700, size: 12pt, fill: ink)[#d.total],
  )
]

#v(4pt)
#align(right)[
  #text(7.5pt, fill: muted)[每列小計四捨五入至整數元，未稅合計為各列相加。]
]

// ---------- 簽署 ----------
//
// 留在第一頁。客戶要簽的是報價內容，簽名欄跟金額分開兩頁會很怪。

#v(20pt)
#grid(
  columns: (1fr, 1fr),
  column-gutter: 24pt,
  ..([報價單位], [客戶簽章]).map(label => [
    #text(8.5pt, fill: muted)[#label]
    #v(28pt)
    #line(length: 100%, stroke: 0.6pt + rgb("#8a9aa6"))
  ]),
)

// ---------- 條款 ----------
//
// 另起一頁。條款佔的篇幅遠大於報價本身，混在同一頁會把金額擠到角落，
// 而客戶真正要看的是金額。

#if d.terms != "" [
  #pagebreak()
  #text(11pt, weight: 700, fill: ink)[報價條款]
  #h(8pt)
  #text(8pt, fill: muted)[#d.quote_no 第 #d.version 版]
  #v(3pt)
  #line(length: 100%, stroke: 0.6pt + ink)
  #v(8pt)
  #block(width: 100%)[
    #set text(8.2pt, fill: rgb("#44555f"))
    #set par(leading: 0.68em, justify: true)
    #d.terms
  ]
]
