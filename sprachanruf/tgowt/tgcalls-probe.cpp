// Beweist, dass tgcalls auf der N950 nicht nur uebersetzt, sondern laeuft.
//
// Es wird kein Anruf aufgebaut -- dazu braucht es die Gegenstelle und die
// Signalisierung. Was hier zaehlt: die Bibliothek laesst sich binden, die
// Fabriken sind registriert, und die Threads von WebRTC starten auf einem
// Kern von 2010 ohne zu stolpern.

#include <cstdio>
#include <chrono>

#include <tgcalls/Instance.h>
#include <tgcalls/InstanceImpl.h>
#include <tgcalls/v2/InstanceV2Impl.h>
#include <tgcalls/StaticThreads.h>
#include <rtc_base/thread.h>

int main() {
    // Die Fassungen tragen sich nicht von selbst ein -- der Aufrufer
    // sagt, welche er mitgebaut hat. Telegram Desktop macht es genauso.
    tgcalls::Register<tgcalls::InstanceImpl>();
    tgcalls::Register<tgcalls::InstanceV2Impl>();

    auto fassungen = tgcalls::Meta::Versions();
    std::printf("Fassungen (%zu):", fassungen.size());
    for (auto const &f : fassungen) {
        std::printf(" %s", f.c_str());
    }
    std::printf("\n");

    // Die drei Threads, auf denen WebRTC alles abwickelt. Sie zu starten
    // ist der erste Schritt jedes Anrufs -- und der erste, der auf einem
    // Kern von 2010 schiefgehen koennte.
    auto beginn = std::chrono::steady_clock::now();
    auto faeden = tgcalls::StaticThreads::getThreads();
    faeden->getMediaThread()->BlockingCall([] {});
    faeden->getWorkerThread()->BlockingCall([] {});
    faeden->getNetworkThread()->BlockingCall([] {});
    auto dauer = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - beginn).count();
    std::printf("Threads laufen, %lld ms\n", (long long)dauer);
    return 0;
}
