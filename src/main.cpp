#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <winhttp.h>
#include <iostream>
#include <string>
#include <vector>
#include <thread>

#pragma comment(lib, "ws2_32.lib")
#pragma comment(lib, "winhttp.lib")

static std::wstring widen(const std::string& s) {
    return std::wstring(s.begin(), s.end());
}

static std::string narrow(const std::wstring& s) {
    std::string out(s.size(), '\0');
    for (size_t i = 0; i < s.size(); ++i) out[i] = static_cast<char>(s[i]);
    return out;
}

static bool doh_forward(const std::wstring& doh_host, const std::wstring& doh_path,
                         const uint8_t* query, int query_len,
                         std::vector<uint8_t>& answer) {
    HINTERNET session = WinHttpOpen(L"doh-stub/1.0",
        WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0);
    if (!session) return false;

    HINTERNET connect = WinHttpConnect(session, doh_host.c_str(), INTERNET_DEFAULT_HTTPS_PORT, 0);
    if (!connect) { WinHttpCloseHandle(session); return false; }

    HINTERNET request = WinHttpOpenRequest(connect, L"POST", doh_path.c_str(),
        nullptr, WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES, WINHTTP_FLAG_SECURE);
    if (!request) { WinHttpCloseHandle(connect); WinHttpCloseHandle(session); return false; }

    std::wstring headers = L"Content-Type: application/dns-message\r\nAccept: application/dns-message";
    WinHttpAddRequestHeaders(request, headers.c_str(), static_cast<DWORD>(-1), WINHTTP_ADDREQ_FLAG_ADD);

    bool ok = WinHttpSendRequest(request, WINHTTP_NO_ADDITIONAL_HEADERS, 0,
        const_cast<uint8_t*>(query), query_len, query_len, 0) != FALSE;

    if (ok) ok = WinHttpReceiveResponse(request, nullptr) != FALSE;

    if (ok) {
        DWORD avail = 0;
        while (WinHttpQueryDataAvailable(request, &avail) && avail > 0) {
            std::vector<uint8_t> chunk(avail);
            DWORD read = 0;
            if (!WinHttpReadData(request, chunk.data(), avail, &read)) break;
            answer.insert(answer.end(), chunk.begin(), chunk.begin() + read);
        }
    }

    WinHttpCloseHandle(request);
    WinHttpCloseHandle(connect);
    WinHttpCloseHandle(session);
    return ok && !answer.empty();
}

static void handle_query(SOCKET sock, sockaddr_in client_addr, int client_len,
                          std::vector<uint8_t> query,
                          std::wstring doh_host, std::wstring doh_path) {
    std::vector<uint8_t> answer;
    if (doh_forward(doh_host, doh_path, query.data(), static_cast<int>(query.size()), answer)) {
        sendto(sock, reinterpret_cast<const char*>(answer.data()), static_cast<int>(answer.size()), 0,
            reinterpret_cast<sockaddr*>(&client_addr), client_len);
        std::cout << "resolved, " << answer.size() << " bytes back" << std::endl;
    } else {
        std::cout << "doh request failed" << std::endl;
    }
}

int main(int argc, char** argv) {
    int port = 5300;
    std::wstring doh_host = L"cloudflare-dns.com";
    std::wstring doh_path = L"/dns-query";

    for (int i = 1; i < argc; ++i) {
        std::string arg = argv[i];
        if (arg == "--port" && i + 1 < argc) {
            port = std::stoi(argv[++i]);
        } else if (arg == "--doh-host" && i + 1 < argc) {
            doh_host = widen(argv[++i]);
        }
    }

    WSADATA wsa;
    if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0) {
        std::cerr << "WSAStartup failed" << std::endl;
        return 1;
    }

    SOCKET sock = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    if (sock == INVALID_SOCKET) {
        std::cerr << "socket() failed" << std::endl;
        WSACleanup();
        return 1;
    }

    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    inet_pton(AF_INET, "127.0.0.1", &addr.sin_addr);
    addr.sin_port = htons(static_cast<uint16_t>(port));

    if (bind(sock, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == SOCKET_ERROR) {
        std::cerr << "bind() failed on port " << port << " (need admin for 53)" << std::endl;
        WSACleanup();
        return 1;
    }

    std::cout << "doh-stub listening on 127.0.0.1:" << port
        << ", forwarding to " << narrow(doh_host) << std::endl;

    std::vector<uint8_t> buf(512);
    while (true) {
        sockaddr_in client_addr{};
        int client_len = sizeof(client_addr);
        int n = recvfrom(sock, reinterpret_cast<char*>(buf.data()), static_cast<int>(buf.size()), 0,
            reinterpret_cast<sockaddr*>(&client_addr), &client_len);
        if (n <= 0) continue;

        std::vector<uint8_t> query(buf.begin(), buf.begin() + n);
        std::thread(handle_query, sock, client_addr, client_len, query, doh_host, doh_path).detach();
    }

    closesocket(sock);
    WSACleanup();
    return 0;
}
