# Структура репозитория

Статус: **предлагаемая начальная структура**, без преждевременного создания пустых пакетов.

```text
QADREN/
├── README.md
├── AGENTS.md
├── docs/
│   ├── README.md
│   ├── architecture/
│   │   ├── overview.md
│   │   ├── backup-model.md
│   │   ├── nodes-and-delegation.md
│   │   ├── storage-adapters.md
│   │   └── technology-stack.md
│   ├── decisions/
│   ├── development/
│   ├── policies/
│   └── roadmap.md
├── core/             # Rust workspace; создавать при начале кода
├── platform/macos/   # SwiftUI app + iCloud adapter
└── tests/            # общие сценарии при необходимости
```

Это карта владения ответственностями, не требование заранее заполнить все директории. Можно объединить crates при отсутствии самостоятельной ценности разделения. Не плодить bootstrap boilerplate.

## Направление зависимостей

- UI -> тонкий слой команд/состояния -> Core.
- Core -> абстрактный контракт Storage Adapter, но не конкретный macOS UI.
- macOS iCloud implementation -> Apple frameworks, с адаптацией к контракту Core.
- Яндекс adapter -> официальный API; не зависит от SwiftUI.
- Тесты -> контракт и поведение, а не внутренние детали UI.

## Источник истины

Документация находится в **этом** репозитории и версионируется с кодом. Внешний AI-контроллер может ссылаться на неё, но не должен поддерживать альтернативный источник правил QADREN.
