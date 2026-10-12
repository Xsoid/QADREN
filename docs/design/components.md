# QADREN — библиотека компонентов (SwiftUI contract)

См. [tokens](tokens.json), [систему виджетов](widget-system.md) и [safety](../policies/data-safety.md). Имена здесь — **целевые** имена компонентов, а не заявление, что они уже есть в коде. Codex обязан сначала найти существующие аналоги и переиспользовать их.

## Слои

- Tokens/theme: `QDTheme`, `QDColorRole`, `QDTypography`, `QDSpacing`.
- Primitives: `QDIconButton`, `QDStatusLabel`, `QDBadge`, `QDMetric`, `QDProgress`, `QDPathLabel`, `QDEmptyState`, `QDNotice`, `QDConfirm`, `QDFilterChip`.
- Compose: `QDWidgetFrame` + header/body/footer, `QDDataRow`, `QDJobRow`, `QDConnectionRow`, `QDConflictRow`, `QDInspector`.
- Features: `QDMonitoringWorkspace`, `QDWidgetCatalog`, `QDLayoutEditor`, `QDSettingsWorkspace`; реализации представлений конкретных виджетов.

SwiftUI layout/composition и Core/state bridge разделены: ни один visual component не владеет собственной очередью или исполняет файловый I/O.

## 1. Примитивы

| Компонент / contract | Состояния, варианты | Поведение и ограничения |
|---|---|---|
| `QDIconButton(action, symbol, label)` | default, hover, pressed, focused, disabled, busy; secondary/primary/destructive | 28×28 pt предпочтительно; title/VoiceOver label; контекстное действие; disabled не кликабелен; pending не выдавать за выполненную команду |
| `QDStatusLabel(kind, text, evidence)` | ready, active, waiting, paused, needsAction, failed, unknown, offline, unsupported | иконка+текст+семантический цвет; UNKNOWN — требует сверки, не «ошибка»; запрет «успех» только по косвенному отчёту |
| `QDBadge(count, severity)` | neutral/info/warning/danger/unknown | маленькая числовая метка; нет значения — не рисовать 0; критичность не определяется количеством |
| `QDMetric(value?, unit, freshness, provenance)` | loaded, stale, unknown, inaccessible | числа с monospacedDigit, единицы; nil → «—», не вымышленный 0; tooltip с источником и временем |
| `QDProgress(completed?, total?, verified, state)` | determinate/indeterminate/paused/unknown/completed | проценты только при достоверных total/completed; нет ETA, если неизвестен; полный 100% не означает verified copy |
| `QDPathLabel(path, scope)` | truncated, full | полный путь в tooltip/inspector/copy; для опасной операции полный путь виден до подтверждения |
| `QDFilterChip(filter, onChange)` | inherited, overridden, disabled | явно виден override, сброс к фильтру панели, доступность клавиатуры |
| `QDNotice(kind, text, action?)` | info/warning/error/unknown | встроен в контекст; важное сообщение нельзя скрыть «успехом» |
| `QDEmptyState(reason, action?)` | genuinelyEmpty, loading, unsupported, noPermission, filtered, offline, failure | различать «ничего нет» и «не можем прочитать»; action только доступное |
| `QDConfirm(scope, operation, consequences)` | pending, declined, approved, expired | default-deny, название, источник/назначение, полный путь, количество/байты если известны, последствия; подтверждение ограничено scope и временем |

Не создавать собственные поля ввода/поповеры там, где системный `Button/Toggle/Menu/TextField/Picker/ConfirmationDialog/Alert` уже обеспечивает accessibility; брендирование — через styles и компоновку.

## 2. Семантика runtime и evidence

Видимый статус и статус получения данных — **две независимые оси**.

1. Fetch/availability: `loading | fresh | stale(lastSeen) | offline(lastSeen?) | unsupported(reason) | error(reason)`.
2. Job/execution: `queued | active | paused | awaitingConsent | conflict | failed | unknown | completed(reported/providerConfirmed/verified)`.

Плашка «Проверено» разрешена только при достаточном evidence для данного типа операции. `providerConfirmed` и `workerReported` показываются разными подписями. UNKNOWN не включает безусловную кнопку «Повторить»; сначала read-only reconciliation. Offline не равен FAILED.

Управление выполняется по цепочке: UI intent → проверка доступности/права → существующий FFI command → durable Core state → UI refresh. Оптимистическое отображение команды допустимо только как «Отправлено/Ожидает применения», не как завершённый Job.

## 3. Сложные ряды

**Job row:** короткое имя; source → destination; операция/путь; state/evidence; проверенный progress файлов и байтов; активные кнопки pause/resume/cancel только если Core поддерживает их для данного статуса. Контекстное меню «Подробности», «Открыть историю», «Копировать ID»; опасные команды должны проходить Core Policy.

**Connection row:** собственное имя подключения; тип адаптера; connected/offline/authNeeded/unsupported; квота только по данным адаптера с `lastUpdated`; точный идентификатор instance. Несколько подключений к одному provider — отдельные строки. Кнопка «Обновить метаданные» не должна скачивать содержимое.

**Conflict row:** два endpoints + путь, какие атрибуты реально сопоставлены, what-is-unknown, scope правила. Кнопки «Подробнее» и «Решить» лишь при реализованном безопасном resolver. Никакого «разрешить все» без подтверждения границ и явных последствий.

## 4. `QDWidgetFrame`: унифицированная оболочка

Структура строго: `frame → header → content → optional footer → contextual overlay`.

| Часть | Состав | UX |
|---|---|---|
| Frame | фон, тонкая рамка, clip, focus | единый radius 8pt; нормальное состояние без glow |
| Header (32pt) | 14pt icon, title 13pt semibold, optional count, trailing actions | меню «…», refresh, expand; действия могут быть скрыты до hover, но VoiceOver/keyboard доступны |
| Content | собственный layout по типу и density | скролл только при содержимом; без nested scroll-jank; пустота занимает осмысленную высоту |
| Footer | свежесть, units, дополнительные действия | не дублировать глобальный statusbar |
| Overlay | loading/stale/disabled, selected/edit handles | сообщение перекрывает **только** недоступную область и не скрывает critical actions |

При hover допускается усилить border; выбранный в edit-mode widget рисует focus stroke + resize handles. При warning/error подсвечивать узкую зону заголовка/рамки, не превращать весь widget в красную светящуюся панель.

### Контекстное меню виджета

`Refresh` конкретного провайдера → `Density` (compact/regular/expanded) → `Filters` (наследовать/override) → `Expand` → `Open in window` (при поддержке) → `Remove from dashboard` (только edit-mode). Независимость от исполняемых Jobs. Если действие не поддерживается — disabled + объяснение, без пустого меню.

## 5. Инспектор, меню и подтверждения

Инспектор открывается поверх/рядом с текущей рабочей областью без навигационного перехода; на узком окне — sheet, на большом — временная детальная панель или отдельное окно. Закрытие возвращает фокус в выбранный row. Для опасных решений — отдельный sheet/confirmation, где кнопка отказа/закрытия не влияет на Job. Нельзя автоматически выдавать диалог по каждому event; indicator собирает pending items, пользователь открывает их осознанно.

Все элементы имеют правильные `accessibilityLabel`, `accessibilityValue`, `accessibilityHint`; ход Job озвучивается без спама, возможность управлять с клавиатуры обязательна.

## 6. Матрица визуальных состояний (минимум)

| Контекст | Оформление | Основное действие |
|---|---|---|
| Working + fresh | cyan action/progress, нейтральная рамка | pause/open при реальной доступности |
| Verified success | зелёный короткий маркер без навязчивой подсветки | inspect |
| Waiting | нейтральный/синий, не крутить бесконечную анимацию | inspect/reorder если Core позволяет |
| Needs consent / conflict | янтарная рамка и alert icon | inspect → scoped confirm |
| Failed | красный маркер, подробность причины | inspect, retry only if safe |
| UNKNOWN | сиреневый/нейтральный distinct marker | read-only reconciliation |
| Offline/stale | приглушить данные, написать «обновлено …» | refresh metadata when safe |
| Unsupported | нейтральная disable-область + причина | open settings |

Набор является базовым: при добавлении виджета нельзя изобретать несовместимое поведение для уже определённых состояний.
