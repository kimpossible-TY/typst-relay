# Tinymist 개인 포크 개발 계획

작성: 2026-09-19. 작업 디렉터리: `~/Developer/tinymist`.

## 목표와 기준선

큰 수학 문서, CeTZ/Fletcher 및 로컬 패키지를 사용하는 편집에서 의미 색상을 유지하면서 분석 지연, 불필요한 컴파일, 요청 누적을 줄인다. 문서의 컴파일 의미는 유지한다.

- Private development repository: https://github.com/kimpossible-TY/tinymist-flow
- Initial public fork (not used for private development pushes): https://github.com/kimpossible-TY/tinymist
- Upstream: https://github.com/Myriad-Dreamin/tinymist
- 최초 개발 기준: 0.15.8 실행 파일의 소스 커밋 `32f908199ee17ea295512bbc27166e890c438175`.
- 최초 브랜치: `perf/static-import-discovery`.
- 이전 측정: 미호출 함수의 `import calc: max`가 semantic tokens 요청에서 문서 전체 trace를 유발했다. 0.15.2 약 0.001–0.019초, 0.15.4/0.15.8 약 5.2초. 이는 의도적인 고비용 레이아웃을 넣은 최소 예제이며 일반 문서 성능 보장은 아니다.
- 실제 책의 동일 저장본: 0.15.2 0.215초; 0.15.4/0.15.8은 각각 20/25초 관측 한도에서 미완료.
- 관련 이슈: [#2392](https://github.com/Myriad-Dreamin/tinymist/issues/2392), [#2410](https://github.com/Myriad-Dreamin/tinymist/issues/2410), [#529](https://github.com/Myriad-Dreamin/tinymist/issues/529).

## 1차 개발: 실행 없는 의존성 사전 조사

`crates/tinymist-query/src/analysis/pdg.rs`의 의존성 사전 조사가 `analyze_import`를 통해 동적 trace를 실행하지 않도록 바꾼다. 기존 상수 해석으로 확인되는 경로를 수집하고, 나머지는 미해결 상태로 유지한다. 표현식 분석의 기존 정적 모듈 해석과 늦게 발견되는 의존성/SCC 병합 기능을 활용한다.

미호출 함수의 AST는 계속 조사한다. 함수 본문을 건너뛰거나 식별자 `calc`를 문맥 없이 내장 모듈로 간주하지 않는다. 순환 import, 상대 경로, include, 나중에 해석되는 import가 기존 의미를 유지하는지 검증한다.

이 수정은 의존성 사전 조사에서 실행을 제거하는 첫 단계다. 표현식 분석의 다른 동적 fallback까지 모두 제거하거나, 모든 semantic tokens 요청의 실행 횟수를 0으로 보장하는 변경은 아니다. 이 경계를 첫 결과 보고에도 명시한다.

### 검증

1. 같은 소스/툴체인/빌드 프로필로 무수정 및 수정 서버를 빌드한다.
2. 기존 analyzer 단위·스냅샷 테스트와 순환 의존성 테스트를 실행한다. 스냅샷 변경은 검토 후에만 수락한다.
3. 최소 예제와 대조군에 실제 LSP semantic tokens 요청을 보낸다. 응답이 비어 있지 않고 미호출 내장 import 때문에 발생하던 동적 분석 호출이 사라지는지 확인한다.
4. 같은 저장본의 실제 책을 별도 프로세스에서 비교한다. 원본 문서/패키지 캐시/사용 중인 편집기 설정을 변경하지 않는다.
5. formatting, 관련 crate clippy 및 CLI/e2e를 실행하고 제한 사항을 기록한다.

시간 목표는 최소 예제 100ms, 실제 책 반복 요청 p95 300ms로 시작하지만 하드웨어 조건을 기록하고 목표와 실측을 구분한다. 서버 최초 요청과 반복 요청은 별도로 측정한다. 테스트용 CPU 작업을 동시에 실행해 벤치마크를 오염시키지 않는다.

`didOpen`에 따른 정상 진단 컴파일은 별도로 실행될 수 있다. `analyze_expr` 카운터는 동적 trace에 대한 지표이며, 0회라고 해서 모든 문서 컴파일이 금지되었다는 뜻은 아니다. LSP 경과 시간에는 정상 컴파일과의 자원 경쟁도 포함된다. 검증 도구는 `semanticTokens=enable`, `syntaxOnly=disable`로 실행한다.

## 이후 단계

1. **정적 해석 강화와 실행 정책 분리:** 내장 모듈, 별칭, 필드 접근의 기존 정적 해석을 재사용/확장한다. 의미 색상의 전체 호출 경로에서 동적 실행 정책을 전달한다. 미해결 식별자는 보수적으로 분류하며 정확도 변화를 테스트한다. 호버까지 무조건 같은 정책을 적용하지 않는다.
2. **취소와 작업 대기열:** 문서 버전별 중복 작업 공유, 오래된 대기 작업 취소, `$/cancelRequest` 처리. 변경 통지는 빠짐없이 반영한다. CPU 작업의 제한된 풀과 프로토콜 처리를 분리한다. 타임아웃이나 `spawn_blocking`이 실행 중 컴파일을 중단한다고 가정하지 않는다.
3. **패키지/증분 캐시:** 변경 없는 외부 패키지 분석 재사용, 내용과 의존성 기반 무효화. 로컬 패키지는 버전 문자열만으로 불변 취급하지 않는다.
4. **메모리:** 오래된 revision/컴파일 결과의 수명과 캐시별 보유량 계측, 크기/유휴 시간 제한. 캐시 축소에 따른 CPU 재계산 비용을 함께 평가한다.
5. **프리뷰:** 연속 입력의 최신 버전 우선 처리, 선택 가능한 지연 갱신, 로컬 HTTP/WebSocket과 원격 전달을 분리 계측. 강제 중단이 꼭 필요한 경우에만 별도 프로세스를 검토한다.

## 운영과 유지보수

- 일반적인 수정과 개인 정책을 독립된 변경으로 유지한다. upstream 병합 시 동일 회귀 테스트를 반복한다.
- 초기에는 기존 VS Code 확장과 사용자 지정 `tinymist.serverPath`를 이용한다. 실행 호스트에 맞는 바이너리를 사용한다.
- 자동 설치/설정 전환은 검증된 바이너리가 준비된 후 별도 단계로 진행한다.
- 공개 저장소에는 일반화된 재현 예제만 둔다. 개인 문서와 로컬 측정 원본은 git에 추가하지 않는다.
- 세부 동작 계약과 1차 작업 상태는 `openspec/changes/static-import-discovery/`에서 관리한다.

## 1차 개발 결과 — 2026-09-19

의존성 사전 조사의 동적 trace 제거를 구현하고 검증했다. 최소 예제의 의미 색상 응답은 5.55초에서 3.5ms로 줄었으며 토큰 배열/범례가 일치했다. 실제 책 `main.typ` 최초 요청은 기준 서버에서 25초 내 미완료, 수정 서버에서 0.23초였다. 이는 단일 측정이며 p95나 장시간 편집 성능 결과는 아니다.

분석 테스트 74개, CLI/LSP 통합 테스트 9개, 검증 도구 테스트 5개와 query crate의 엄격한 clippy가 통과했다. 의존성까지 포함한 clippy는 변경하지 않은 macOS 코드의 기존 unsafe 주석 오류로 실패했으며 상세 기록에 남겼다.

수정 바이너리는 `target/release/tinymist`와 `.local/candidate/tinymist`에 있다. 현재 편집기 연결 전환과 원격 프리뷰 실사용 검증은 아직 하지 않았다. [세부 검증 결과](openspec/changes/static-import-discovery/validation.md)를 참고한다.

## 첫 패치 보존 및 실제 환경 적용 — 2026-09-19

첫 패치를 `288b215d`로 커밋하고 비공개 저장소 `kimpossible-TY/tinymist-flow`에 업로드했다. 실제 원격 VS Code의 Tinymist 서버를 검증된 수정 바이너리로 전환했으며, 의미 색상을 다시 활성화한 설정 적용까지 서버 로그로 확인했다. 위 1차 개발 시점의 미적용 상태는 이 배포로 갱신되었다. 장시간 편집과 원격 프리뷰 실사용 검증은 다음 단계로 남는다.

[배포 내역과 원복 방법](openspec/changes/static-import-discovery/deployment.md)에 설치 위치, 체크섬, 원본 백업을 기록했다.
